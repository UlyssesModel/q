//! Reconstruct `(rho_re, rho_im)` from the flat complex density matrix.

use num_complex::Complex32;

pub fn rho_to_real_imag(rho: &[Complex32]) -> (Vec<f32>, Vec<f32>) {
    let mut re = Vec::with_capacity(rho.len());
    let mut im = Vec::with_capacity(rho.len());
    for c in rho {
        re.push(c.re);
        im.push(c.im);
    }
    (re, im)
}
