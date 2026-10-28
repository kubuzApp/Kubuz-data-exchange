//! Subset osifont to the characters the drawings use and print a Rust table.
fn main(){
 let p=std::env::args().nth(1).unwrap(); let out=std::env::args().nth(2).unwrap();
 let data=std::fs::read(&p).unwrap();
 let face=ttf_parser::Face::parse(&data,0).unwrap();
 let upm=face.units_per_em() as f64;
 let k=1000.0/upm;
 let chars: Vec<char> = (32u32..127).chain(160..256).chain([0x2013,0x2014,0x2019,0x2030,0x2300,0x2205,0x25A1,0x2220,0x2312,0x2313,0x20AC].into_iter()).filter_map(char::from_u32).collect();
 let mut m=subsetter::GlyphRemapper::new();
 let mut rows=vec![];
 for c in &chars { if let Some(g)=face.glyph_index(*c){ let ng=m.remap(g.0); let adv=(face.glyph_hor_advance(g).unwrap_or(0) as f64*k).round() as u16; rows.push((*c as u32, ng, adv)); } }
 let r=subsetter::subset(&data,0,&m).unwrap();
 std::fs::write(format!("{out}/osifont-subset.ttf"),&r).unwrap();
 let bb=face.global_bounding_box();
 let mut s=String::new();
 s.push_str("//! Generated from osifont-lgpl3fe.ttf by the font tool in the repo's\n//! `tools/` folder: the subset's glyph ids and advance widths (per 1000\n//! em) for every character it carries, sorted by code point.\n\n");
 s.push_str(&format!("pub const ASCENT: i32 = {};\npub const DESCENT: i32 = {};\npub const CAP_HEIGHT: i32 = {};\npub const BBOX: [i32; 4] = [{}, {}, {}, {}];\npub const GLYPH_COUNT: u16 = {};\n\n",
   (face.ascender() as f64*k).round(), (face.descender() as f64*k).round(), (face.capital_height().unwrap_or(0) as f64*k).round(),
   (bb.x_min as f64*k).round(),(bb.y_min as f64*k).round(),(bb.x_max as f64*k).round(),(bb.y_max as f64*k).round(), m.num_gids()));
 s.push_str("/// (code point, glyph id in the subset, advance width).\npub const GLYPHS: &[(u32, u16, u16)] = &[\n");
 for (c,g,a) in &rows { s.push_str(&format!("    (0x{c:04X}, {g}, {a}),\n")); }
 s.push_str("];\n");
 std::fs::write(format!("{out}/glyphs.rs"), s).unwrap();
 println!("{} chars, {} glyphs, {} bytes", rows.len(), m.num_gids(), r.len());
}
