//! Small shared helpers.

const KIB: f64 = 1024.0;

/// 1_288_490_189 -> "1.2 GB". Binary units, one decimal above KB.
pub fn fmt_bytes(bytes: u64) -> String {
    let b = bytes as f64;
    if b < KIB {
        return format!("{bytes} B");
    }
    let units = ["KB", "MB", "GB", "TB"];
    let mut value = b / KIB;
    let mut i = 0;
    while value >= KIB && i < units.len() - 1 {
        value /= KIB;
        i += 1;
    }
    format!("{value:.1} {}", units[i])
}

/// Least-squares fit over `(x, y)` points. Returns `(slope, r_squared)`; `None` for < 2 points or
/// zero x-variance.
pub fn linear_fit(points: &[(f64, f64)]) -> Option<(f64, f64)> {
    let n = points.len() as f64;
    if points.len() < 2 {
        return None;
    }
    let mx = points.iter().map(|p| p.0).sum::<f64>() / n;
    let my = points.iter().map(|p| p.1).sum::<f64>() / n;
    let (mut sxx, mut sxy, mut syy) = (0.0, 0.0, 0.0);
    for &(x, y) in points {
        sxx += (x - mx) * (x - mx);
        sxy += (x - mx) * (y - my);
        syy += (y - my) * (y - my);
    }
    if sxx == 0.0 {
        return None;
    }
    let r2 = if syy == 0.0 {
        1.0
    } else {
        (sxy * sxy) / (sxx * syy)
    };
    Some((sxy / sxx, r2))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_bytes() {
        assert_eq!(fmt_bytes(0), "0 B");
        assert_eq!(fmt_bytes(1023), "1023 B");
        assert_eq!(fmt_bytes(1536), "1.5 KB");
        assert_eq!(fmt_bytes(1_288_490_189), "1.2 GB");
        assert_eq!(fmt_bytes(5 * 1024 * 1024 * 1024 * 1024), "5.0 TB");
        assert!(fmt_bytes(u64::MAX).ends_with("TB"));
    }

    #[test]
    fn linear_fit_slope_and_r2() {
        let (s, r2) = linear_fit(&[(0.0, 1.0), (1.0, 3.0), (2.0, 5.0)]).unwrap();
        assert!((s - 2.0).abs() < 1e-9 && (r2 - 1.0).abs() < 1e-9);
        let (s, r2) = linear_fit(&[(0.0, 5.0), (1.0, 5.0), (2.0, 5.0)]).unwrap();
        assert_eq!((s, r2), (0.0, 1.0));
        let (_, r2) = linear_fit(&[(0.0, 0.0), (1.0, 10.0), (2.0, 0.0), (3.0, 10.0)]).unwrap();
        assert!((r2 - 0.2).abs() < 1e-9);
    }

    #[test]
    fn linear_fit_degenerate() {
        assert!(linear_fit(&[]).is_none());
        assert!(linear_fit(&[(1.0, 1.0)]).is_none());
        assert!(linear_fit(&[(1.0, 1.0), (1.0, 2.0)]).is_none());
    }
}
