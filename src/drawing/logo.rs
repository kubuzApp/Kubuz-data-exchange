//! The owner's logo as SVG path data (`M L H V C S Q T Z`, absolute or
//! relative), drawn as filled PDF paths. Arcs (`A`) are not read: flatten
//! them to curves when exporting the SVG. The logo is scaled to fit its
//! cell and keeps its proportions.

use crate::pdf::Content;
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Logo {
    /// The SVG viewBox width and height the paths are drawn in.
    pub width: f64,
    pub height: f64,
    pub paths: Vec<LogoPath>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogoPath {
    pub d: String,
    /// `#rrggbb`; black when missing.
    #[serde(default)]
    pub fill: Option<String>,
}

#[derive(Clone, Copy, Debug)]
enum Cmd {
    Move(f64, f64),
    Line(f64, f64),
    Cubic(f64, f64, f64, f64, f64, f64),
    Close,
}

/// Draw `logo` filling a `w` × `h` box whose lower-left corner is `(x, y)`
/// on the sheet, centred in it.
pub fn draw(logo: &Logo, c: &mut Content, x: f64, y: f64, w: f64, h: f64) -> Result<(), String> {
    if logo.width <= 0.0 || logo.height <= 0.0 {
        return Err("logo: width and height must be positive".into());
    }
    let k = (w / logo.width).min(h / logo.height);
    let ox = x + (w - logo.width * k) / 2.0;
    let oy = y + (h + logo.height * k) / 2.0;
    // SVG y runs down; the sheet's runs up.
    let map = |p: (f64, f64)| [ox + p.0 * k, oy - p.1 * k];
    for path in &logo.paths {
        let cmds = parse(&path.d)?;
        c.save();
        match &path.fill {
            Some(hex) => c.fill_rgb(rgb(hex)?),
            None => c.fill_gray(0.0),
        }
        for cmd in cmds {
            match cmd {
                Cmd::Move(a, b) => c.move_to(map((a, b))),
                Cmd::Line(a, b) => c.line_to(map((a, b))),
                Cmd::Cubic(a, b, d, e, f, g) => c.curve_to(map((a, b)), map((d, e)), map((f, g))),
                Cmd::Close => c.close(),
            }
        }
        c.fill_even_odd();
        c.restore();
    }
    Ok(())
}

fn rgb(hex: &str) -> Result<[f64; 3], String> {
    let h = hex.trim().trim_start_matches('#');
    if h.len() != 6 || !h.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!("logo: fill {hex:?} is not #rrggbb"));
    }
    let ch = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).unwrap();
    Ok([
        f64::from(ch(0)) / 255.0,
        f64::from(ch(2)) / 255.0,
        f64::from(ch(4)) / 255.0,
    ])
}

/// Absolute commands from SVG path data; quadratics become cubics.
fn parse(d: &str) -> Result<Vec<Cmd>, String> {
    let mut out = Vec::new();
    let toks = tokens(d)?;
    let mut i = 0;
    let (mut cx, mut cy) = (0.0, 0.0);
    let (mut sx, mut sy) = (0.0, 0.0);
    let mut last_c: Option<(f64, f64)> = None;
    let mut last_q: Option<(f64, f64)> = None;
    let mut cmd = ' ';
    while i < toks.len() {
        if let Tok::Cmd(ch) = toks[i] {
            cmd = ch;
            i += 1;
            if cmd == 'Z' || cmd == 'z' {
                out.push(Cmd::Close);
                cx = sx;
                cy = sy;
                last_c = None;
                last_q = None;
                continue;
            }
        }
        let rel = cmd.is_ascii_lowercase();
        let up = cmd.to_ascii_uppercase();
        let need = match up {
            'M' | 'L' | 'T' => 2,
            'H' | 'V' => 1,
            'C' => 6,
            'S' | 'Q' => 4,
            'A' => {
                return Err("logo: elliptical arcs (A) are not read; flatten them to curves".into())
            }
            _ => return Err(format!("logo: unknown path command {cmd:?}")),
        };
        let mut v = [0.0; 6];
        for slot in v.iter_mut().take(need) {
            match toks.get(i) {
                Some(Tok::Num(n)) => *slot = *n,
                _ => return Err(format!("logo: path command {cmd} needs {need} numbers")),
            }
            i += 1;
        }
        let (ox, oy) = if rel { (cx, cy) } else { (0.0, 0.0) };
        match up {
            'M' => {
                cx = ox + v[0];
                cy = oy + v[1];
                sx = cx;
                sy = cy;
                out.push(Cmd::Move(cx, cy));
                // Further pairs after M are implicit L.
                cmd = if rel { 'l' } else { 'L' };
                last_c = None;
                last_q = None;
            }
            'L' => {
                cx = ox + v[0];
                cy = oy + v[1];
                out.push(Cmd::Line(cx, cy));
                last_c = None;
                last_q = None;
            }
            'H' => {
                cx = ox + v[0];
                out.push(Cmd::Line(cx, cy));
                last_c = None;
                last_q = None;
            }
            'V' => {
                cy = oy + v[0];
                out.push(Cmd::Line(cx, cy));
                last_c = None;
                last_q = None;
            }
            'C' => {
                let p = (
                    ox + v[0],
                    oy + v[1],
                    ox + v[2],
                    oy + v[3],
                    ox + v[4],
                    oy + v[5],
                );
                out.push(Cmd::Cubic(p.0, p.1, p.2, p.3, p.4, p.5));
                last_c = Some((p.2, p.3));
                last_q = None;
                cx = p.4;
                cy = p.5;
            }
            'S' => {
                let c1 = match last_c {
                    Some((px, py)) => (2.0 * cx - px, 2.0 * cy - py),
                    None => (cx, cy),
                };
                let p = (ox + v[0], oy + v[1], ox + v[2], oy + v[3]);
                out.push(Cmd::Cubic(c1.0, c1.1, p.0, p.1, p.2, p.3));
                last_c = Some((p.0, p.1));
                last_q = None;
                cx = p.2;
                cy = p.3;
            }
            'Q' | 'T' => {
                let (qx, qy, ex, ey) = if up == 'Q' {
                    (ox + v[0], oy + v[1], ox + v[2], oy + v[3])
                } else {
                    let q = match last_q {
                        Some((px, py)) => (2.0 * cx - px, 2.0 * cy - py),
                        None => (cx, cy),
                    };
                    (q.0, q.1, ox + v[0], oy + v[1])
                };
                let c1 = (cx + 2.0 / 3.0 * (qx - cx), cy + 2.0 / 3.0 * (qy - cy));
                let c2 = (ex + 2.0 / 3.0 * (qx - ex), ey + 2.0 / 3.0 * (qy - ey));
                out.push(Cmd::Cubic(c1.0, c1.1, c2.0, c2.1, ex, ey));
                last_q = Some((qx, qy));
                last_c = None;
                cx = ex;
                cy = ey;
            }
            _ => unreachable!(),
        }
    }
    Ok(out)
}

#[derive(Clone, Copy, Debug)]
enum Tok {
    Cmd(char),
    Num(f64),
}

fn tokens(d: &str) -> Result<Vec<Tok>, String> {
    let mut out = Vec::new();
    let b: Vec<char> = d.chars().collect();
    let mut i = 0;
    while i < b.len() {
        let ch = b[i];
        if ch.is_ascii_alphabetic() {
            out.push(Tok::Cmd(ch));
            i += 1;
        } else if ch.is_ascii_digit() || ch == '-' || ch == '+' || ch == '.' {
            let start = i;
            i += 1;
            while i < b.len() {
                let c = b[i];
                let prev = b[i - 1];
                let cont = c.is_ascii_digit()
                    || (c == '.'
                        && !b[start..i].contains(&'.')
                        && !b[start..i].iter().any(|x| *x == 'e' || *x == 'E'))
                    || ((c == 'e' || c == 'E')
                        && !b[start..i].iter().any(|x| *x == 'e' || *x == 'E'))
                    || ((c == '-' || c == '+') && (prev == 'e' || prev == 'E'));
                if !cont {
                    break;
                }
                i += 1;
            }
            let s: String = b[start..i].iter().collect();
            out.push(Tok::Num(
                s.parse::<f64>()
                    .map_err(|_| format!("logo: bad number {s:?} in path data"))?,
            ));
        } else if ch.is_whitespace() || ch == ',' {
            i += 1;
        } else {
            return Err(format!("logo: unexpected {ch:?} in path data"));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_relative_and_shorthand_commands() {
        let cmds = parse("M10 10 h20v10 l-5,5 q5 5 10 0 s5-5 0-10 z").unwrap();
        assert!(matches!(cmds[0], Cmd::Move(x, y) if x == 10.0 && y == 10.0));
        assert!(matches!(cmds[1], Cmd::Line(x, y) if x == 30.0 && y == 10.0));
        assert!(matches!(cmds[2], Cmd::Line(x, y) if x == 30.0 && y == 20.0));
        assert!(matches!(cmds[4], Cmd::Cubic(..)));
        assert!(matches!(cmds.last(), Some(Cmd::Close)));
        assert!(parse("M0 0 A5 5 0 0 1 10 10").is_err());
        assert!(parse("M0 0 L1").is_err());
    }

    #[test]
    fn draws_scaled_into_the_cell() {
        let logo = Logo {
            width: 100.0,
            height: 50.0,
            paths: vec![LogoPath {
                d: "M0 0H100V50H0Z".into(),
                fill: Some("#336699".into()),
            }],
        };
        let mut c = Content::new();
        draw(&logo, &mut c, 10.0, 10.0, 40.0, 40.0).unwrap();
        // 100 × 50 fits 40 wide at 0.4: 40 × 20, centred vertically at 20..40.
        assert!(c.as_str().contains("1000 4000 m"), "{}", c.as_str());
        assert!(c.as_str().contains("5000 2000 l"));
        assert!(c.as_str().contains("0.2 0.4 0.6 rg"));
    }
}
