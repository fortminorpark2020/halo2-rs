//! The inverse modified discrete cosine transform, through a complex FFT a
//! quarter of the output's size.

use std::f64::consts::PI;

#[derive(Clone, Copy, Default)]
struct Complex {
    re: f64,
    im: f64,
}

impl Complex {
    fn mul(self, o: Complex) -> Complex {
        Complex {
            re: self.re * o.re - self.im * o.im,
            im: self.re * o.im + self.im * o.re,
        }
    }

    fn turn(angle: f64) -> Complex {
        Complex {
            re: angle.cos(),
            im: angle.sin(),
        }
    }
}

/// IMDCT of `n` coefficients to `2n` samples, matching FFmpeg's full
/// inverse MDCT (`AV_TX_FULL_IMDCT`) with the given scale.
pub struct Imdct {
    n: usize,
    scale: f64,
    pre: Vec<Complex>,
    post: Vec<Complex>,
    /// FFT twiddles for the n/2 point transform.
    roots: Vec<Complex>,
    reverse: Vec<usize>,
    work: Vec<Complex>,
    dct: Vec<f64>,
}

impl Imdct {
    pub fn new(n: usize, scale: f64) -> Imdct {
        let m = n / 2;
        let bits = m.trailing_zeros();
        Imdct {
            n,
            scale,
            pre: (0..m)
                .map(|k| Complex::turn(-PI * (k as f64 + 0.25) / n as f64))
                .collect(),
            post: (0..m)
                .map(|j| Complex::turn(-PI * j as f64 / n as f64))
                .collect(),
            roots: (0..m / 2)
                .map(|k| Complex::turn(-2.0 * PI * k as f64 / m as f64))
                .collect(),
            reverse: (0..m)
                .map(|i| {
                    if bits == 0 {
                        0
                    } else {
                        i.reverse_bits() >> (usize::BITS - bits)
                    }
                })
                .collect(),
            work: vec![Complex::default(); m],
            dct: vec![0.0; n],
        }
    }

    /// The DCT-IV of the input: sum of x[k] cos(pi/n (i + 1/2)(k + 1/2)).
    fn dct4(&mut self, x: &[f32]) {
        let n = self.n;
        let m = n / 2;
        for k in 0..m {
            let z = Complex {
                re: x[2 * k] as f64,
                im: x[n - 1 - 2 * k] as f64,
            };
            self.work[self.reverse[k]] = z.mul(self.pre[k]);
        }
        // Radix-2 FFT, in place.
        let mut size = 2;
        while size <= m {
            let step = m / size;
            for start in (0..m).step_by(size) {
                for i in 0..size / 2 {
                    let a = self.work[start + i];
                    let b = self.work[start + i + size / 2].mul(self.roots[i * step]);
                    self.work[start + i] = Complex {
                        re: a.re + b.re,
                        im: a.im + b.im,
                    };
                    self.work[start + i + size / 2] = Complex {
                        re: a.re - b.re,
                        im: a.im - b.im,
                    };
                }
            }
            size *= 2;
        }
        for j in 0..m {
            let s = self.work[j].mul(self.post[j]);
            self.dct[2 * j] = s.re;
            self.dct[n - 1 - 2 * j] = -s.im;
        }
    }

    /// `out` gets 2n samples.
    pub fn run(&mut self, input: &[f32], out: &mut [f32]) {
        let n = self.n;
        let h = n / 2;
        self.dct4(&input[..n]);
        let s = self.scale;
        for i in 0..h {
            out[i] = (-s * self.dct[h + i]) as f32;
            out[h + i] = (s * self.dct[n - 1 - i]) as f32;
            out[n + i] = (s * self.dct[h - 1 - i]) as f32;
            out[n + h + i] = (s * self.dct[i]) as f32;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// FFmpeg's reference: a half IMDCT unfolded to the full output.
    fn naive(x: &[f32], scale: f64) -> Vec<f32> {
        let n = x.len();
        let len = n / 2;
        let phase = PI / (4.0 * n as f64);
        let mut half = vec![0.0f64; n];
        for i in 0..len {
            let i_d = phase * (4 * len - 2 * i - 1) as f64;
            let i_u = phase * (3 * n + 2 * i + 1) as f64;
            let (mut d, mut u) = (0.0, 0.0);
            for (j, &v) in x.iter().enumerate() {
                let a = (2 * j + 1) as f64;
                d += (a * i_d).cos() * v as f64;
                u += (a * i_u).cos() * v as f64;
            }
            half[i] = d * scale;
            half[i + len] = -u * scale;
        }
        let mut out = vec![0.0f64; 2 * n];
        out[n / 2..n / 2 + n].copy_from_slice(&half);
        for i in 0..n / 2 {
            out[i] = -out[n - i - 1];
            out[2 * n - i - 1] = out[n + i];
        }
        out.into_iter().map(|v| v as f32).collect()
    }

    #[test]
    fn matches_the_direct_formula() {
        for n in [16usize, 128, 256] {
            let x: Vec<f32> = (0..n)
                .map(|i| ((i * 7919) % 101) as f32 - 50.0 + (i as f32 * 0.37).sin() * 20.0)
                .collect();
            let mut fast = Imdct::new(n, 1.0 / 32768.0);
            let mut out = vec![0.0; 2 * n];
            fast.run(&x, &mut out);
            let want = naive(&x, 1.0 / 32768.0);
            for (a, b) in out.iter().zip(&want) {
                assert!((a - b).abs() < 1e-5, "{n}: {a} vs {b}");
            }
        }
    }
}
