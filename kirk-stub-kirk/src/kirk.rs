//! `Kirk` — stateful builder-constructed compute handle.
//!
//! The bon-derived builder owns a small set of scalar parameters and a handful
//! of pre-allocated `ndarray::Array2` buffers sized off `visible_nodes`. The
//! five public methods consume an `ArrayView2<Complex64>` and return tuple
//! outputs whose shapes are functions of the input only. Output values are
//! straightforward row / column reductions of the input.

use bon::Builder;
use getset::{Getters, Setters};
use ndarray::{Array1, Array2, ArrayView2, Axis};
use num_complex::Complex64;
use num_traits::Zero;
use rand::{Rng, SeedableRng};
use rand_xoshiro::Xoshiro256StarStar;

/// Public configuration / state struct. Construct via [`Kirk::builder`] or
/// [`Kirk::new`].
#[derive(Builder, Clone, Debug, Getters, Setters)]
#[builder(derive(Debug))]
#[builder(builder_type(name = KirkBuilder, vis = "pub"))]
#[builder(finish_fn(name = build_internal, vis = ""))]
#[builder(state_mod(name = kirk_builder, vis = "pub"))]
#[getset(get = "pub", set = "pub")]
pub struct Kirk {
    #[builder(default = 1.0e-3)]
    pub learning_rate: f64,

    #[builder(default = 1.0)]
    pub tau: f64,

    #[builder(default = 1.0e-4)]
    pub cooling_rate: f64,

    #[builder(default = 8)]
    pub visible_nodes: usize,

    pub seed: Option<u64>,

    #[builder(default = true)]
    pub enforce_symmetry: bool,

    #[builder(default = true)]
    pub enforce_purity: bool,

    #[builder(default = false)]
    pub potential_field_active: bool,

    #[builder(default = 0.0)]
    pub v_phi_loc: f64,

    #[builder(default = 1.0)]
    pub v_phi_scale: f64,

    #[builder(default = -1.0e-3)]
    pub rho_hat_init_min: f64,

    #[builder(default = 1.0e-3)]
    pub rho_hat_init_max: f64,

    #[builder(skip)]
    pub rho_hat: Array2<Complex64>,

    #[builder(skip)]
    pub hidden_bool_inter: Array2<bool>,

    #[builder(skip)]
    pub hidden_bool_intra: Array2<bool>,

    #[builder(skip)]
    pub rho_t: Array2<Complex64>,

    #[builder(skip)]
    pub hamiltonian: Array2<Complex64>,

    #[builder(skip)]
    pub obserable: Array2<Complex64>,
}

impl<S: kirk_builder::IsComplete> KirkBuilder<S> {
    /// Finalize the builder, populating the skip-built buffers with the right
    /// shapes derived from `visible_nodes`.
    pub fn build(self) -> Kirk {
        let mut k = self.build_internal();
        let v = k.visible_nodes;
        let total = v.saturating_mul(2);
        // Deterministic seeded fill if a seed is supplied, otherwise zeros.
        k.rho_hat = if let Some(s) = k.seed {
            let mut rng = Xoshiro256StarStar::seed_from_u64(s);
            let lo = k.rho_hat_init_min;
            let hi = k.rho_hat_init_max.max(lo);
            let mut out = Array2::<Complex64>::zeros((total, total));
            for elem in out.iter_mut() {
                let x: f64 = rng.gen_range(lo..=hi);
                *elem = Complex64::new(x, 0.0);
            }
            out
        } else {
            Array2::<Complex64>::zeros((total, total))
        };
        k.hidden_bool_inter = Array2::<bool>::from_elem((v, v), false);
        k.hidden_bool_intra = Array2::<bool>::from_elem((v, v), false);
        k.rho_t = Array2::<Complex64>::zeros((total, total));
        k.hamiltonian = Array2::<Complex64>::zeros((total, total));
        k.obserable = Array2::<Complex64>::zeros((total, total));
        k
    }
}

impl Default for Kirk {
    fn default() -> Self {
        Kirk::builder().build()
    }
}

impl Kirk {
    /// Convenience constructor — equivalent to [`Kirk::builder().build()`].
    pub fn new() -> Kirk {
        Kirk::builder().build()
    }

    /// Full variant: returns `(array NxN, vector 2N, scalar, scalar f64)`.
    /// Advances the internal `tau` by `cooling_rate`.
    pub fn active_inference(
        &mut self,
        sample: ArrayView2<'_, Complex64>,
    ) -> (Array2<Complex64>, Array1<Complex64>, Complex64, f64) {
        let (arr, vec, scalar) = self.inference_features(sample);
        let ent = self.inference_entropy(sample);
        self.tau += self.cooling_rate;
        (arr, vec, scalar, ent)
    }

    /// Same outputs as [`Kirk::inference_features`]; advances `tau` by
    /// `cooling_rate`.
    pub fn active_inference_features(
        &mut self,
        sample: ArrayView2<'_, Complex64>,
    ) -> (Array2<Complex64>, Array1<Complex64>, Complex64) {
        let out = self.inference_features(sample);
        self.tau += self.cooling_rate;
        out
    }

    /// Same output as [`Kirk::inference_entropy`]; advances `tau` by
    /// `cooling_rate`.
    pub fn active_inference_entropy(&mut self, sample: ArrayView2<'_, Complex64>) -> f64 {
        let e = self.inference_entropy(sample);
        self.tau += self.cooling_rate;
        e
    }

    /// Returns a non-negative f64 derived deterministically from `sample`.
    pub fn inference_entropy(&mut self, sample: ArrayView2<'_, Complex64>) -> f64 {
        let n = sample.nrows().max(1);
        sample.iter().map(|c| c.norm_sqr()).sum::<f64>() / ((n * n) as f64)
    }

    /// Returns `(NxN array, 2N vector, scalar)` derived from `sample`.
    pub fn inference_features(
        &mut self,
        sample: ArrayView2<'_, Complex64>,
    ) -> (Array2<Complex64>, Array1<Complex64>, Complex64) {
        let n = sample.nrows();
        let arr = sample.to_owned();
        let row_means = sample
            .mean_axis(Axis(1))
            .unwrap_or_else(|| Array1::<Complex64>::zeros(n));
        let col_means = sample
            .mean_axis(Axis(0))
            .unwrap_or_else(|| Array1::<Complex64>::zeros(n));
        let mut vec = Array1::<Complex64>::zeros(2 * n);
        for i in 0..n {
            vec[i] = row_means[i];
            vec[n + i] = col_means[i];
        }
        let scalar = sample.mean().unwrap_or_else(Complex64::zero);
        (arr, vec, scalar)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ndarray::Array2;

    fn make_sample(n: usize) -> Array2<Complex64> {
        // Deterministic NxN complex sample with non-trivial values.
        let mut a = Array2::<Complex64>::zeros((n, n));
        for i in 0..n {
            for j in 0..n {
                let r = (i as f64 + 1.0) * 0.5 - (j as f64) * 0.25;
                let im = (j as f64) * 0.125 - (i as f64) * 0.0625;
                a[[i, j]] = Complex64::new(r, im);
            }
        }
        a
    }

    #[test]
    fn builder_default_shapes() {
        let k = Kirk::builder().visible_nodes(4).build();
        assert_eq!(k.rho_hat.shape(), &[8, 8]);
        assert_eq!(k.hidden_bool_inter.shape(), &[4, 4]);
        assert_eq!(k.hidden_bool_intra.shape(), &[4, 4]);
        assert_eq!(k.rho_t.shape(), &[8, 8]);
        assert_eq!(k.hamiltonian.shape(), &[8, 8]);
        assert_eq!(k.obserable.shape(), &[8, 8]);
    }

    #[test]
    fn builder_seeded_rho_hat_is_deterministic() {
        let a = Kirk::builder().visible_nodes(4).seed(42u64).build();
        let b = Kirk::builder().visible_nodes(4).seed(42u64).build();
        assert_eq!(a.rho_hat, b.rho_hat);
    }

    #[test]
    fn builder_unseeded_rho_hat_is_zero() {
        let k = Kirk::builder().visible_nodes(4).build();
        for v in k.rho_hat.iter() {
            assert_eq!(*v, Complex64::zero());
        }
    }

    #[test]
    fn new_default_constructs() {
        let k = Kirk::new();
        let d = Kirk::default();
        assert_eq!(k.visible_nodes, d.visible_nodes);
    }

    fn shape_finite_for(n: usize) {
        let s = make_sample(n);
        let mut k = Kirk::builder().visible_nodes(n).seed(7u64).build();
        let (arr, vec, scalar, ent) = k.active_inference(s.view());
        assert_eq!(arr.shape(), &[n, n]);
        assert_eq!(vec.shape(), &[2 * n]);
        assert!(scalar.re.is_finite() && scalar.im.is_finite());
        assert!(ent.is_finite() && ent >= 0.0);
        for c in arr.iter() {
            assert!(c.re.is_finite() && c.im.is_finite());
        }
        for c in vec.iter() {
            assert!(c.re.is_finite() && c.im.is_finite());
        }
    }

    #[test]
    fn active_inference_shape_n2() {
        shape_finite_for(2);
    }

    #[test]
    fn active_inference_shape_n4() {
        shape_finite_for(4);
    }

    #[test]
    fn active_inference_shape_n8() {
        shape_finite_for(8);
    }

    #[test]
    fn inference_features_shapes_only() {
        let n = 6;
        let s = make_sample(n);
        let mut k = Kirk::builder().visible_nodes(n).build();
        let (arr, vec, scalar) = k.inference_features(s.view());
        assert_eq!(arr.shape(), &[n, n]);
        assert_eq!(vec.shape(), &[2 * n]);
        assert!(scalar.re.is_finite() && scalar.im.is_finite());
    }

    #[test]
    fn inference_entropy_finite_and_nonnegative() {
        let s = make_sample(5);
        let mut k = Kirk::builder().visible_nodes(5).build();
        let e = k.inference_entropy(s.view());
        assert!(e.is_finite());
        assert!(e >= 0.0);
    }

    #[test]
    fn inference_entropy_zero_on_zero_input() {
        let n = 4;
        let s = Array2::<Complex64>::zeros((n, n));
        let mut k = Kirk::builder().visible_nodes(n).build();
        assert_eq!(k.inference_entropy(s.view()), 0.0);
    }

    #[test]
    fn deterministic_given_input() {
        let s = make_sample(4);
        let mut a = Kirk::builder().visible_nodes(4).seed(11u64).build();
        let mut b = Kirk::builder().visible_nodes(4).seed(11u64).build();
        let oa = a.inference_features(s.view());
        let ob = b.inference_features(s.view());
        assert_eq!(oa.0, ob.0);
        assert_eq!(oa.1, ob.1);
        assert_eq!(oa.2, ob.2);
        assert_eq!(a.inference_entropy(s.view()), b.inference_entropy(s.view()));
    }

    #[test]
    fn tau_advances_on_active_variants() {
        let s = make_sample(3);
        let mut k = Kirk::builder().visible_nodes(3).cooling_rate(0.5).build();
        let tau0 = k.tau;
        let _ = k.active_inference(s.view());
        let tau1 = k.tau;
        let _ = k.active_inference_features(s.view());
        let tau2 = k.tau;
        let _ = k.active_inference_entropy(s.view());
        let tau3 = k.tau;
        assert!((tau1 - tau0 - 0.5).abs() < 1e-12);
        assert!((tau2 - tau1 - 0.5).abs() < 1e-12);
        assert!((tau3 - tau2 - 0.5).abs() < 1e-12);
    }

    #[test]
    fn non_active_variants_do_not_advance_tau() {
        let s = make_sample(3);
        let mut k = Kirk::builder().visible_nodes(3).cooling_rate(0.5).build();
        let tau0 = k.tau;
        let _ = k.inference_features(s.view());
        let _ = k.inference_entropy(s.view());
        assert_eq!(k.tau, tau0);
    }
}
