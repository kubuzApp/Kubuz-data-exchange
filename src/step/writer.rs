//! A numbered entity list for the DATA section of an ISO 10303-21 file,
//! with the AP214 entities this crate uses.

use std::fmt::Write as _;

use crate::view::P3;

/// A number the way a STEP reader wants it: short, and never an integer.
pub fn f(x: f64) -> String {
    let s = format!("{x:.9}");
    let s = s.trim_end_matches('0').to_string();
    if s.ends_with('.') {
        s + "0"
    } else {
        s
    }
}

pub fn join(ids: &[usize]) -> String {
    ids.iter()
        .map(|i| format!("#{i}"))
        .collect::<Vec<_>>()
        .join(",")
}

/// A STEP string: single quotes doubled, and no control characters.
pub fn quote(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_control())
        .flat_map(|c| if c == '\'' { vec!['\'', '\''] } else { vec![c] })
        .collect()
}

fn norm(v: P3) -> f64 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

pub fn unit(v: P3) -> P3 {
    let n = norm(v);
    [v[0] / n, v[1] / n, v[2] / n]
}

pub fn sub(a: P3, b: P3) -> P3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: P3, b: P3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

pub fn cross(a: P3, b: P3) -> P3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// Any unit vector at right angles to `z`.
pub fn any_perp(z: P3) -> P3 {
    let t = if z[0].abs() < 0.9 {
        [1.0, 0.0, 0.0]
    } else {
        [0.0, 1.0, 0.0]
    };
    unit(cross(cross(z, t), z))
}

#[derive(Default)]
pub struct StepWriter {
    body: String,
    n: usize,
}

impl StepWriter {
    pub fn add(&mut self, e: String) -> usize {
        self.n += 1;
        writeln!(self.body, "#{}={};", self.n, e).unwrap();
        self.n
    }

    pub fn point(&mut self, p: P3) -> usize {
        self.add(format!(
            "CARTESIAN_POINT('',({},{},{}))",
            f(p[0]),
            f(p[1]),
            f(p[2])
        ))
    }

    pub fn dir(&mut self, d: P3) -> usize {
        let d = unit(d);
        self.add(format!(
            "DIRECTION('',({},{},{}))",
            f(d[0]),
            f(d[1]),
            f(d[2])
        ))
    }

    /// Placement at `o` with axis `z` and reference direction `x` (made
    /// perpendicular to `z` here).
    pub fn axis(&mut self, o: P3, z: P3, x: P3) -> usize {
        let z = unit(z);
        let k = dot(x, z);
        let x = unit([x[0] - z[0] * k, x[1] - z[1] * k, x[2] - z[2] * k]);
        let p = self.point(o);
        let dz = self.dir(z);
        let dx = self.dir(x);
        self.add(format!("AXIS2_PLACEMENT_3D('',#{p},#{dz},#{dx})"))
    }

    pub fn line(&mut self, a: P3, b: P3) -> usize {
        let p = self.point(a);
        let d = self.dir(sub(b, a));
        let v = self.add(format!("VECTOR('',#{d},{})", f(norm(sub(b, a)))));
        self.add(format!("LINE('',#{p},#{v})"))
    }

    pub fn circle(&mut self, c: P3, axis: P3, refd: P3, r: f64) -> usize {
        let ax = self.axis(c, axis, refd);
        self.add(format!("CIRCLE('',#{ax},{})", f(r)))
    }

    pub fn plane(&mut self, p: P3, n: P3) -> usize {
        let ax = self.axis(p, n, any_perp(unit(n)));
        self.add(format!("PLANE('',#{ax})"))
    }

    pub fn cylinder(&mut self, c: P3, axis: P3, refd: P3, r: f64) -> usize {
        let ax = self.axis(c, axis, refd);
        self.add(format!("CYLINDRICAL_SURFACE('',#{ax},{})", f(r)))
    }

    pub fn bspline_curve(
        &mut self,
        degree: usize,
        ctrl: &[P3],
        knots: &[f64],
        mults: &[usize],
    ) -> usize {
        let pts: Vec<usize> = ctrl.iter().map(|&c| self.point(c)).collect();
        self.add(format!(
            "B_SPLINE_CURVE_WITH_KNOTS('',{degree},({}),.UNSPECIFIED.,.F.,.F.,({}),({}),.UNSPECIFIED.)",
            join(&pts),
            mults
                .iter()
                .map(|x| x.to_string())
                .collect::<Vec<_>>()
                .join(","),
            knots.iter().map(|&x| f(x)).collect::<Vec<_>>().join(",")
        ))
    }

    /// `degree` along the two curves, degree 1 from `a` to `b`. Both curves
    /// share `knots` and `mults`.
    pub fn ruled_bspline(
        &mut self,
        degree: usize,
        a: &[P3],
        b: &[P3],
        knots: &[f64],
        mults: &[usize],
    ) -> usize {
        let rows: Vec<String> = a
            .iter()
            .zip(b)
            .map(|(&p, &q)| {
                let (i, j) = (self.point(p), self.point(q));
                format!("(#{i},#{j})")
            })
            .collect();
        self.add(format!(
            "B_SPLINE_SURFACE_WITH_KNOTS('',{degree},1,({}),.UNSPECIFIED.,.F.,.F.,.F.,({}),(2,2),({}),(0.,1.),.UNSPECIFIED.)",
            rows.join(","),
            mults
                .iter()
                .map(|x| x.to_string())
                .collect::<Vec<_>>()
                .join(","),
            knots.iter().map(|&x| f(x)).collect::<Vec<_>>().join(",")
        ))
    }

    pub fn oriented(&mut self, e: usize, fwd: bool) -> usize {
        self.add(format!(
            "ORIENTED_EDGE('',*,*,#{e},{})",
            if fwd { ".T." } else { ".F." }
        ))
    }

    pub fn face(&mut self, bounds: &[usize], surf: usize, same_sense: bool) -> usize {
        self.add(format!(
            "ADVANCED_FACE('',({}),#{surf},{})",
            join(bounds),
            if same_sense { ".T." } else { ".F." }
        ))
    }

    /// The whole file round the entities written so far.
    pub fn finish(self, name: &str) -> String {
        format!(
            "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION(('Kubuz part'),'2;1');\n\
             FILE_NAME('{}.step','',(''),(''),'kubuz-step','Kubuz','');\n\
             FILE_SCHEMA(('AUTOMOTIVE_DESIGN {{ 1 0 10303 214 1 1 1 1 }}'));\nENDSEC;\nDATA;\n{}ENDSEC;\nEND-ISO-10303-21;\n",
            quote(name),
            self.body
        )
    }
}
