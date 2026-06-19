//! tonic service implementation. Field translation between the protobuf
//! `Matrix`/`ComplexMatrix` (raw little-endian f32 bytes) and the
//! `kirk-stub-realistic` `Vec<f32>` / `Vec<Complex32>` types.

use std::sync::Arc;
use std::time::Instant;

use num_complex::Complex32;
use tonic::{Request, Response, Status};

use crate::backend::KirkBackend;
use crate::error::ServerError;
use crate::metrics::MetricsHandle;

use super::proto::kirk_service_server::KirkService;
use super::proto::{
    ActiveInferenceResponse, ComplexMatrix, EntropyResponse, FeatureScalar, FeaturesResponse,
    KirkRequest, KirkResponse, Matrix, SampleRequest, SampleResponse, SampleSizeRequest,
};

pub struct KirkSvc {
    pub backend: Arc<KirkBackend>,
    pub metrics: MetricsHandle,
}

fn err_to_status(err: ServerError) -> Status {
    use ServerError::*;
    match err {
        BadRequest(m) => Status::invalid_argument(m),
        PayloadTooLarge { .. } => Status::resource_exhausted(err.to_string()),
        MatrixDimExceeded { .. } => Status::invalid_argument(err.to_string()),
        Compute(_) => Status::internal(err.to_string()),
        Shutdown => Status::unavailable("server shutting down"),
        Internal(m) => Status::internal(m),
    }
}

fn decode_le_f32(bytes: &[u8], expected_len: usize) -> Result<Vec<f32>, ServerError> {
    let expected = expected_len * 4;
    if bytes.is_empty() && expected_len > 0 {
        // Treat empty as zeros — convenient for real-only callers (`data_im=[]`).
        return Ok(vec![0.0f32; expected_len]);
    }
    if bytes.len() != expected {
        return Err(ServerError::BadRequest(format!(
            "matrix bytes len {} != expected {}",
            bytes.len(),
            expected
        )));
    }
    let mut out = Vec::with_capacity(expected_len);
    for chunk in bytes.chunks_exact(4) {
        let b = [chunk[0], chunk[1], chunk[2], chunk[3]];
        out.push(f32::from_le_bytes(b));
    }
    Ok(out)
}

fn complex_matrix(arr: &[Complex32], dim: u32) -> ComplexMatrix {
    let mut re = Vec::with_capacity(arr.len() * 4);
    let mut im = Vec::with_capacity(arr.len() * 4);
    for c in arr {
        re.extend_from_slice(&c.re.to_le_bytes());
        im.extend_from_slice(&c.im.to_le_bytes());
    }
    ComplexMatrix {
        dim,
        data_re: re,
        data_im: im,
    }
}

#[tonic::async_trait]
impl KirkService for KirkSvc {
    async fn forward(
        &self,
        request: Request<KirkRequest>,
    ) -> Result<Response<KirkResponse>, Status> {
        let t0 = Instant::now();
        let req = request.into_inner();
        let res = async {
            let m: Matrix = req
                .matrix
                .ok_or_else(|| ServerError::BadRequest("missing matrix".into()))?;
            let n = m.dim;
            self.backend.check_dim(n)?;
            let len = (n as usize) * (n as usize);
            let re = decode_le_f32(&m.data_re, len)?;
            let im = decode_le_f32(&m.data_im, len)?;
            let out = self
                .backend
                .clone()
                .forward(re, im, n as usize, req.timestamp_us)
                .await?;
            let rho_complex: Vec<Complex32> = out
                .matrix_re
                .iter()
                .zip(&out.matrix_im)
                .map(|(r, i)| Complex32::new(*r, *i))
                .collect();
            Ok::<_, ServerError>(KirkResponse {
                entropy_re: out.entropy_re,
                entropy_im: out.entropy_im,
                entropy: out.entropy,
                entropy_zscore: out.entropy_zscore,
                regime: out.regime,
                confidence: out.confidence,
                processing_time_us: out.processing_time_us,
                timestamp_us: out.timestamp_us,
                rho: Some(complex_matrix(&rho_complex, n)),
            })
        }
        .await;
        let ok = res.is_ok();
        self.metrics
            .observe("grpc", "forward", t0.elapsed().as_micros() as f64, ok);
        res.map(Response::new).map_err(err_to_status)
    }

    async fn inference_entropy(
        &self,
        request: Request<SampleRequest>,
    ) -> Result<Response<EntropyResponse>, Status> {
        let t0 = Instant::now();
        let req = request.into_inner();
        let res = async {
            let s = req
                .sample
                .ok_or_else(|| ServerError::BadRequest("missing sample".into()))?;
            self.backend.check_dim(s.dim)?;
            let len = (s.dim as usize) * (s.dim as usize);
            let re = decode_le_f32(&s.data_re, len)?;
            let im = decode_le_f32(&s.data_im, len)?;
            let e = self
                .backend
                .clone()
                .inference_entropy(re, im, s.dim as usize)
                .await?;
            Ok::<_, ServerError>(EntropyResponse {
                total_relative_entropy: e,
            })
        }
        .await;
        let ok = res.is_ok();
        self.metrics.observe(
            "grpc",
            "inference_entropy",
            t0.elapsed().as_micros() as f64,
            ok,
        );
        res.map(Response::new).map_err(err_to_status)
    }

    async fn inference_features(
        &self,
        request: Request<SampleRequest>,
    ) -> Result<Response<FeaturesResponse>, Status> {
        let t0 = Instant::now();
        let req = request.into_inner();
        let res = async {
            let s = req
                .sample
                .ok_or_else(|| ServerError::BadRequest("missing sample".into()))?;
            self.backend.check_dim(s.dim)?;
            let len = (s.dim as usize) * (s.dim as usize);
            let re = decode_le_f32(&s.data_re, len)?;
            let im = decode_le_f32(&s.data_im, len)?;
            let f = self
                .backend
                .clone()
                .inference_features(re, im, s.dim as usize)
                .await?;
            Ok::<_, ServerError>(features_proto(f, s.dim))
        }
        .await;
        let ok = res.is_ok();
        self.metrics.observe(
            "grpc",
            "inference_features",
            t0.elapsed().as_micros() as f64,
            ok,
        );
        res.map(Response::new).map_err(err_to_status)
    }

    async fn active_inference(
        &self,
        request: Request<SampleRequest>,
    ) -> Result<Response<ActiveInferenceResponse>, Status> {
        let t0 = Instant::now();
        let req = request.into_inner();
        let res = async {
            let s = req
                .sample
                .ok_or_else(|| ServerError::BadRequest("missing sample".into()))?;
            self.backend.check_dim(s.dim)?;
            let len = (s.dim as usize) * (s.dim as usize);
            let re = decode_le_f32(&s.data_re, len)?;
            let im = decode_le_f32(&s.data_im, len)?;
            let out = self
                .backend
                .clone()
                .active_inference(re, im, s.dim as usize)
                .await?;
            let f = out.features;
            let n_u32 = s.dim;
            Ok::<_, ServerError>(ActiveInferenceResponse {
                feature_arr: Some(complex_matrix(&f.feature_arr, n_u32)),
                feature_vec: Some(complex_matrix(&f.feature_vec, 2 * n_u32)),
                feature_scalar: Some(FeatureScalar {
                    re: f.feature_scalar.re,
                    im: f.feature_scalar.im,
                }),
                total_relative_entropy: out.total_relative_entropy,
            })
        }
        .await;
        let ok = res.is_ok();
        self.metrics.observe(
            "grpc",
            "active_inference",
            t0.elapsed().as_micros() as f64,
            ok,
        );
        res.map(Response::new).map_err(err_to_status)
    }

    async fn active_inference_entropy(
        &self,
        request: Request<SampleRequest>,
    ) -> Result<Response<EntropyResponse>, Status> {
        // Same compute as inference_entropy in the stub.
        self.inference_entropy(request).await
    }

    async fn active_inference_features(
        &self,
        request: Request<SampleRequest>,
    ) -> Result<Response<FeaturesResponse>, Status> {
        self.inference_features(request).await
    }

    async fn forward_sample(
        &self,
        request: Request<SampleSizeRequest>,
    ) -> Result<Response<SampleResponse>, Status> {
        let t0 = Instant::now();
        let req = request.into_inner();
        let res = async {
            self.backend.check_dim(req.dim)?;
            let out = self
                .backend
                .clone()
                .forward_sample(req.dim as usize, req.seed)
                .await?;
            Ok::<_, ServerError>(SampleResponse {
                feature_array: Some(complex_matrix(&out.feature_array, req.dim)),
                feature_vector: Some(complex_matrix(&out.feature_vector, 2 * req.dim)),
                feature_scalar: Some(FeatureScalar {
                    re: out.feature_scalar.re,
                    im: out.feature_scalar.im,
                }),
                relative_entropy: out.relative_entropy,
            })
        }
        .await;
        let ok = res.is_ok();
        self.metrics.observe(
            "grpc",
            "forward_sample",
            t0.elapsed().as_micros() as f64,
            ok,
        );
        res.map(Response::new).map_err(err_to_status)
    }
}

fn features_proto(f: kirk_stub_realistic::variants::Features, dim: u32) -> FeaturesResponse {
    FeaturesResponse {
        feature_arr: Some(complex_matrix(&f.feature_arr, dim)),
        feature_vec: Some(complex_matrix(&f.feature_vec, 2 * dim)),
        feature_scalar: Some(FeatureScalar {
            re: f.feature_scalar.re,
            im: f.feature_scalar.im,
        }),
    }
}

// re-export for the binary
pub mod export {
    pub use super::super::proto::kirk_service_server::KirkServiceServer;
}
