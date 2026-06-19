pub mod service;

pub mod proto {
    tonic::include_proto!("kirk.v1");
}

pub use service::KirkSvc;
