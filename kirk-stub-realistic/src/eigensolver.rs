//! Stages 1-2: Hermitianize + complex Hermitian eigendecomposition.
//!
//! Strategy (see ADR-001): nalgebra has no native complex Hermitian eigh, so we
//! build the real symmetric `2N x 2N` block matrix
//!
//! ```text
//!     [  Re(H)   -Im(H) ]
//!     [  Im(H)    Re(H) ]
//! ```
//!
//! and diagonalize it with `SymmetricEigen`. Each real eigenvalue appears twice;
//! we take every-other one and reconstruct the complex eigenvector `v = a + i b`
//! from the corresponding `(a, b)` pair of real eigenvector components.

use crate::output::KirkError;
use nalgebra::{DMatrix, SymmetricEigen};
use num_complex::Complex32;

/// Square Hermitian complex matrix in row-major flattened form.
#[derive(Debug, Clone)]
pub struct Hermitian {
    pub dim: usize,
    pub re: Vec<f32>,
    pub im: Vec<f32>,
}

impl Hermitian {
    /// Mirrors Python `hermitianize`: validates shape, builds H = (H_raw + H_raw^†)/2.
    pub fn from_parts(matrix_re: &[f32], matrix_im: &[f32], n: usize) -> Result<Self, KirkError> {
        if matrix_re.len() != matrix_im.len() {
            return Err(KirkError::ShapeMismatch {
                rows_re: n,
                cols_re: matrix_re.len() / n.max(1),
                rows_im: n,
                cols_im: matrix_im.len() / n.max(1),
            });
        }
        if n == 0 {
            return Err(KirkError::Empty);
        }
        if matrix_re.len() != n * n {
            return Err(KirkError::NotSquare {
                dim: n,
                len: matrix_re.len(),
            });
        }
        let mut re = vec![0.0f32; n * n];
        let mut im = vec![0.0f32; n * n];
        for i in 0..n {
            for j in 0..n {
                // (H_raw + H_raw^†) / 2 → element-wise:
                //   Re_sym[i,j] = (Re[i,j] + Re[j,i]) / 2
                //   Im_skew[i,j] = (Im[i,j] - Im[j,i]) / 2
                let r = (matrix_re[i * n + j] + matrix_re[j * n + i]) * 0.5;
                let m = (matrix_im[i * n + j] - matrix_im[j * n + i]) * 0.5;
                re[i * n + j] = r;
                im[i * n + j] = m;
            }
        }
        Ok(Self { dim: n, re, im })
    }

    #[inline]
    pub fn at(&self, i: usize, j: usize) -> Complex32 {
        Complex32::new(self.re[i * self.dim + j], self.im[i * self.dim + j])
    }
}

/// Eigendecomposition output: eigenvalues (ascending) and the matrix of
/// eigenvectors `V` (column-`k` is the eigenvector for `lam[k]`).
#[derive(Debug, Clone)]
pub struct Eigh {
    pub dim: usize,
    pub lam: Vec<f32>,
    /// Column-major store of `V` as complex: `V[i + k*N]` is row i of column k.
    pub v: Vec<Complex32>,
}

impl Eigh {
    #[inline]
    pub fn v_at(&self, i: usize, k: usize) -> Complex32 {
        self.v[i + k * self.dim]
    }
}

/// Diagonalize a complex Hermitian matrix via the real `2N` block trick.
pub fn diagonalize(h: &Hermitian) -> Result<Eigh, KirkError> {
    let n = h.dim;
    if n == 0 {
        return Err(KirkError::Empty);
    }

    // Build the (2N x 2N) real symmetric block matrix. nalgebra uses column-major.
    // We need M[r, c] where:
    //   top-left  (r<n, c<n)   = Re[r,c]
    //   top-right (r<n, c>=n)  = -Im[r, c-n]
    //   bot-left  (r>=n, c<n)  =  Im[r-n, c]
    //   bot-right (r>=n, c>=n) = Re[r-n, c-n]
    let two_n = 2 * n;
    let mut data = vec![0.0f32; two_n * two_n];
    let put = |data: &mut [f32], r: usize, c: usize, v: f32| {
        // column-major
        data[r + c * two_n] = v;
    };
    for r in 0..n {
        for c in 0..n {
            let re = h.re[r * n + c];
            let im = h.im[r * n + c];
            put(&mut data, r, c, re); // top-left
            put(&mut data, r, c + n, -im); // top-right
            put(&mut data, r + n, c, im); // bottom-left
            put(&mut data, r + n, c + n, re); // bottom-right
        }
    }
    let big = DMatrix::<f32>::from_vec(two_n, two_n, data);
    // Symmetric block (verified by construction). Use SymmetricEigen which
    // expects symmetric input and returns real eigenvalues + orthonormal vecs.
    let eig = SymmetricEigen::new(big);
    let eigenvalues = eig.eigenvalues;
    let eigenvectors = eig.eigenvectors;

    // The block matrix's eigenvalues are the Hermitian eigenvalues each doubled
    // (one pair per complex eigenvector). Sort ascending and pick every-other
    // entry. Reconstruct the corresponding complex eigenvector from the
    // (top, bottom) halves: v = top + i * bottom.

    // Sort indices by eigenvalue (ascending).
    let mut order: Vec<usize> = (0..two_n).collect();
    order.sort_by(|&a, &b| {
        eigenvalues[a]
            .partial_cmp(&eigenvalues[b])
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Take the lower N (one of each doubled eigenvalue). We greedily walk the
    // sorted list, skipping pairs that share the same eigenvalue and a degenerate
    // eigenvector basis — for distinct (non-degenerate) eigenvalues this is
    // simply "take every other one".
    let mut lam = Vec::with_capacity(n);
    let mut v_flat: Vec<Complex32> = vec![Complex32::new(0.0, 0.0); n * n];
    let mut k = 0usize; // output column index
    let mut idx = 0usize; // position in sorted order
    while k < n && idx < two_n {
        let i = order[idx];
        let lam_i = eigenvalues[i];

        // Pull the column vector from `eigenvectors`.
        let col = eigenvectors.column(i);
        // Real eigenvector for the block matrix is (a, b) with |a|^2 + |b|^2 = 1.
        // The complex Hermitian eigenvector is v = a + i b (unit-norm under the
        // standard complex inner product since |a|^2 + |b|^2 = 1).
        for r in 0..n {
            v_flat[r + k * n] = Complex32::new(col[r], col[r + n]);
        }
        lam.push(lam_i);
        k += 1;

        // Skip the paired duplicate of this eigenvalue.
        idx += 1;
        if idx < two_n {
            // Walk forward until we find an index whose eigenvalue is meaningfully
            // distinct; otherwise skip one slot (the paired duplicate). Tolerance
            // is conservative — equal-to-many-ulps for degenerate eigenvalues.
            let next_lam = eigenvalues[order[idx]];
            let same = (next_lam - lam_i).abs() <= 1e-5 * lam_i.abs().max(1.0);
            if same {
                idx += 1;
            }
        }
    }

    if lam.len() != n {
        return Err(KirkError::EigenFailure);
    }

    Ok(Eigh {
        dim: n,
        lam,
        v: v_flat,
    })
}
