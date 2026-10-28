//! The one font every drawing uses: a subset of osifont (ISO 3098 type B
//! lettering, see `NOTICE-osifont.md`), embedded once per document as a
//! CIDFontType2 with Identity-H encoding. Text is written as glyph ids, so
//! the width of a string is known here without a font engine.

mod glyphs {
    include!("glyphs.rs");
}

pub use glyphs::{ASCENT, BBOX, CAP_HEIGHT, DESCENT, GLYPHS, GLYPH_COUNT};

/// The embedded TrueType subset.
pub const FONT_DATA: &[u8] = include_bytes!("osifont-subset.ttf");

/// The name the PDF gives the font: a subset tag and the family.
pub const FONT_NAME: &str = "KUBUZO+osifont";

/// Capital height as a fraction of the em: an ISO 3098 lettering height
/// `h` is a capital height, so a font size of `h / CAP` gives it.
pub const CAP: f64 = CAP_HEIGHT as f64 / 1000.0;

/// The glyph id and advance width (per 1000 em) of `c`, or of `?` when
/// the subset has no glyph for it.
pub fn glyph(c: char) -> (u16, u16) {
    let code = c as u32;
    match GLYPHS.binary_search_by_key(&code, |g| g.0) {
        Ok(i) => (GLYPHS[i].1, GLYPHS[i].2),
        Err(_) => glyph('?'),
    }
}

/// Whether the subset carries `c`.
pub fn has(c: char) -> bool {
    GLYPHS.binary_search_by_key(&(c as u32), |g| g.0).is_ok()
}

/// The width of `text` in mm when its capitals are `h` mm tall.
pub fn width(text: &str, h: f64) -> f64 {
    let em = h / CAP;
    text.chars().map(|c| f64::from(glyph(c).1)).sum::<f64>() / 1000.0 * em
}

/// `text` as the hex string the content stream writes: two bytes per
/// glyph id.
pub fn encode(text: &str) -> String {
    let mut s = String::with_capacity(text.len() * 4 + 2);
    s.push('<');
    for c in text.chars() {
        let (g, _) = glyph(c);
        s.push_str(&format!("{g:04X}"));
    }
    s.push('>');
    s
}

/// The `/W` array of the CID font: one entry per glyph, in glyph order.
pub fn widths_array() -> String {
    let mut by_gid: Vec<(u16, u16)> = GLYPHS.iter().map(|g| (g.1, g.2)).collect();
    by_gid.sort_unstable();
    by_gid.dedup();
    let mut s = String::from("[");
    for (g, w) in by_gid {
        s.push_str(&format!("{g} [{w}] "));
    }
    s.push(']');
    s
}

/// The ToUnicode CMap, so text can be searched and copied.
pub fn to_unicode_cmap() -> String {
    let mut s = String::from(
        "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n\
         /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n\
         /CMapName /Adobe-Identity-UCS def\n/CMapType 2 def\n\
         1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n",
    );
    // At most 100 entries per bfchar block, as the CMap spec asks.
    for chunk in GLYPHS.chunks(100) {
        s.push_str(&format!("{} beginbfchar\n", chunk.len()));
        for (code, gid, _) in chunk {
            if *code > 0xFFFF {
                continue;
            }
            s.push_str(&format!("<{gid:04X}> <{code:04X}>\n"));
        }
        s.push_str("endbfchar\n");
    }
    s.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glyph_table_is_sorted_and_covers_the_drawing_symbols() {
        for w in GLYPHS.windows(2) {
            assert!(w[0].0 < w[1].0);
        }
        for c in "0123456789,.-+×⌀°R± ABCabcéèëçÉ".chars() {
            assert!(has(c), "{c}");
        }
        assert!(u16::try_from(GLYPHS.len()).unwrap() < GLYPH_COUNT + 1);
    }

    #[test]
    fn widths_scale_with_the_lettering_height() {
        let w = width("1000", 2.5);
        assert!(w > 5.0 && w < 8.0, "{w}");
        assert!((width("1000", 5.0) - 2.0 * w).abs() < 1e-9);
        assert_eq!(encode("0"), format!("<{:04X}>", glyph('0').0));
    }
}
