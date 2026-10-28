//! A small PDF writer: enough of ISO 32000 to write a PDF/A-2b drawing
//! dossier and nothing more. Objects go into object streams, the cross
//! reference is a stream, every content stream is Flate-compressed, one
//! embedded font serves every page, and each sheet format's frame is a
//! Form XObject drawn by every page of that size.
//!
//! Nothing here knows what a drawing is: [`crate::drawing`] draws into a
//! [`content::Content`] and hands the page over.

use miniz_oxide::deflate::compress_to_vec_zlib;

pub mod content;
pub mod font;

pub use content::Content;

/// What the document says about itself, in the Info dictionary and the
/// XMP packet (PDF/A wants both, and wants them to agree).
#[derive(Clone, Debug)]
pub struct DocInfo {
    pub title: String,
    pub author: String,
    /// The application, e.g. `kubuz-data-exchange 0.2.0`.
    pub creator: String,
    /// `YYYY-MM-DD`; the document carries no clock, so this is its date.
    pub date: String,
    /// ISO 639-1, e.g. `nl`.
    pub lang: String,
}

enum Object {
    Plain(String),
    Stream { dict: String, data: Vec<u8> },
    Unset,
}

struct Page {
    id: u32,
    label: String,
    /// The page dictionary up to (not including) its closing `>>`; empty
    /// while the page is only reserved.
    dict: String,
}

/// An internal link: a rectangle on one page that jumps to another.
struct Link {
    from: usize,
    rect: [f64; 4],
    to: usize,
}

struct Outline {
    title: String,
    page: u32,
    children: Vec<Outline>,
}

pub struct Doc {
    info: DocInfo,
    objects: Vec<Object>,
    font_id: u32,
    resources_id: u32,
    pages_id: u32,
    pages: Vec<Page>,
    forms: Vec<(String, u32)>,
    outline: Vec<Outline>,
    links: Vec<Link>,
}

impl Doc {
    pub fn new(info: DocInfo) -> Self {
        let mut doc = Doc {
            info,
            objects: Vec::new(),
            font_id: 0,
            resources_id: 0,
            pages_id: 0,
            pages: Vec::new(),
            forms: Vec::new(),
            outline: Vec::new(),
            links: Vec::new(),
        };
        doc.pages_id = doc.alloc();
        doc.resources_id = doc.alloc();
        doc.font_id = doc.embed_font();
        doc
    }

    fn alloc(&mut self) -> u32 {
        self.objects.push(Object::Unset);
        self.objects.len() as u32
    }

    fn set(&mut self, id: u32, body: String) {
        self.objects[id as usize - 1] = Object::Plain(body);
    }

    /// A stream object; `extra` are dictionary entries besides Length and
    /// Filter. Compressed unless `raw`.
    fn stream(&mut self, id: u32, extra: &str, data: Vec<u8>, raw: bool) {
        let (dict, data) = if raw {
            (format!("<< {extra} /Length {} >>", data.len()), data)
        } else {
            let z = compress_to_vec_zlib(&data, 6);
            (
                format!("<< {extra} /Filter /FlateDecode /Length {} >>", z.len()),
                z,
            )
        };
        self.objects[id as usize - 1] = Object::Stream { dict, data };
    }

    fn embed_font(&mut self) -> u32 {
        let type0 = self.alloc();
        let cid = self.alloc();
        let desc = self.alloc();
        let file = self.alloc();
        let tounicode = self.alloc();
        let name = font::FONT_NAME;
        self.set(
            type0,
            format!(
                "<< /Type /Font /Subtype /Type0 /BaseFont /{name} /Encoding /Identity-H \
                 /DescendantFonts [{cid} 0 R] /ToUnicode {tounicode} 0 R >>"
            ),
        );
        self.set(
            cid,
            format!(
                "<< /Type /Font /Subtype /CIDFontType2 /BaseFont /{name} \
                 /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> \
                 /FontDescriptor {desc} 0 R /DW 500 /W {} /CIDToGIDMap /Identity >>",
                font::widths_array()
            ),
        );
        let [x0, y0, x1, y1] = font::BBOX;
        self.set(
            desc,
            format!(
                "<< /Type /FontDescriptor /FontName /{name} /Flags 4 \
                 /FontBBox [{x0} {y0} {x1} {y1}] /ItalicAngle 0 /Ascent {} /Descent {} \
                 /CapHeight {} /StemV 80 /FontFile2 {file} 0 R >>",
                font::ASCENT,
                font::DESCENT,
                font::CAP_HEIGHT
            ),
        );
        let data = font::FONT_DATA.to_vec();
        self.stream(file, &format!("/Length1 {}", data.len()), data, false);
        self.stream(tounicode, "", font::to_unicode_cmap().into_bytes(), false);
        type0
    }

    /// A Form XObject every page of one sheet format draws first: the
    /// frame, the title block's lines and captions, the logo. `content` is
    /// in the same hundredths of a millimetre as a page.
    pub fn add_form(&mut self, name: &str, w_mm: f64, h_mm: f64, content: Content) {
        let id = self.alloc();
        let bbox = format!(
            "[0 0 {} {}]",
            (w_mm * content::UNITS_PER_MM).round(),
            (h_mm * content::UNITS_PER_MM).round()
        );
        let extra = format!(
            "/Type /XObject /Subtype /Form /BBox {bbox} /Matrix [0.028346457 0 0 0.028346457 0 0] \
             /Resources << /Font << /F1 {} 0 R >> >>",
            self.font_id
        );
        self.stream(id, &extra, content.into_bytes(), false);
        self.forms.push((name.to_string(), id));
    }

    /// A page of `w_mm` × `h_mm` that draws `forms` first and then
    /// `content`. `label` is what the viewer shows as its page number.
    pub fn add_page(
        &mut self,
        w_mm: f64,
        h_mm: f64,
        forms: &[&str],
        content: Content,
        label: &str,
    ) -> usize {
        let i = self.reserve_page();
        self.fill_page(i, w_mm, h_mm, forms, content, label);
        i
    }

    /// A page slot to fill later with [`Doc::fill_page`], so front matter
    /// can come before the pages it describes. Returns its index.
    pub fn reserve_page(&mut self) -> usize {
        let page = self.alloc();
        self.pages.push(Page {
            id: page,
            label: String::new(),
            dict: String::new(),
        });
        self.pages.len() - 1
    }

    /// Fill the reserved page `i`, as [`Doc::add_page`] would.
    pub fn fill_page(
        &mut self,
        i: usize,
        w_mm: f64,
        h_mm: f64,
        forms: &[&str],
        content: Content,
        label: &str,
    ) {
        let stream = self.alloc();
        let mut body = String::new();
        for f in forms {
            body.push_str(&format!("/{f} Do\n"));
        }
        body.push_str("q\n");
        body.push_str(content::MATRIX);
        body.push('\n');
        body.push_str(content.as_str());
        body.push_str("Q\n");
        self.stream(stream, "", body.into_bytes(), false);
        self.pages[i].dict = format!(
            "<< /Type /Page /Parent {} 0 R /MediaBox [0 0 {} {}] /Resources {} 0 R /Contents {stream} 0 R",
            self.pages_id,
            pt(w_mm),
            pt(h_mm),
            self.resources_id
        );
        self.pages[i].label = label.to_string();
    }

    /// A link on page `from` (0-based) over `rect` (mm, `[x0, y0, x1, y1]`)
    /// that jumps to page `to`. Either page may still be reserved.
    pub fn add_link(&mut self, from: usize, rect: [f64; 4], to: usize) {
        self.links.push(Link { from, rect, to });
    }

    /// The number of pages so far; the next page's index.
    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    /// A bookmark to page `page` (0-based), under `parent` (a path of
    /// titles from the root; empty for a top-level item). A missing parent
    /// is created without a page of its own.
    pub fn add_outline(&mut self, parent: &[&str], title: &str, page: usize) {
        let page_id = self.pages[page].id;
        let mut level = &mut self.outline;
        for p in parent {
            let i = match level.iter().position(|o| o.title == *p) {
                Some(i) => i,
                None => {
                    level.push(Outline {
                        title: p.to_string(),
                        page: page_id,
                        children: Vec::new(),
                    });
                    level.len() - 1
                }
            };
            level = &mut level[i].children;
        }
        level.push(Outline {
            title: title.to_string(),
            page: page_id,
            children: Vec::new(),
        });
    }

    fn write_outlines(&mut self, items: &[Outline], parent: u32, ids: &[u32]) -> usize {
        let mut count = 0;
        for (k, item) in items.iter().enumerate() {
            let id = ids[k];
            let child_ids: Vec<u32> = item.children.iter().map(|_| self.alloc()).collect();
            let sub = self.write_outlines(&item.children, id, &child_ids);
            let mut d = format!(
                "<< /Title {} /Parent {parent} 0 R /Dest [{} 0 R /Fit]",
                pdf_string(&item.title),
                item.page
            );
            if k > 0 {
                d.push_str(&format!(" /Prev {} 0 R", ids[k - 1]));
            }
            if k + 1 < ids.len() {
                d.push_str(&format!(" /Next {} 0 R", ids[k + 1]));
            }
            if let (Some(f), Some(l)) = (child_ids.first(), child_ids.last()) {
                d.push_str(&format!(" /First {f} 0 R /Last {l} 0 R /Count {sub}"));
            }
            d.push_str(" >>");
            self.set(id, d);
            count += 1 + sub;
        }
        count
    }

    /// The whole file.
    pub fn finish(mut self) -> Vec<u8> {
        // Shared resources.
        let mut xo = String::new();
        for (name, id) in &self.forms {
            xo.push_str(&format!("/{name} {id} 0 R "));
        }
        self.set(
            self.resources_id,
            format!(
                "<< /Font << /F1 {} 0 R >> /XObject << {xo}>> >>",
                self.font_id
            ),
        );
        // Link annotations (PDF/A: printable, no appearance of their own,
        // no border), then the page objects.
        let mut annots: Vec<Vec<u32>> = vec![Vec::new(); self.pages.len()];
        let links = std::mem::take(&mut self.links);
        for l in links {
            let id = self.alloc();
            let to = self.pages[l.to].id;
            let [x0, y0, x1, y1] = l.rect;
            self.set(
                id,
                format!(
                    "<< /Type /Annot /Subtype /Link /Rect [{} {} {} {}] /Border [0 0 0] /F 4 \
                     /Dest [{to} 0 R /Fit] >>",
                    pt(x0),
                    pt(y0),
                    pt(x1),
                    pt(y1)
                ),
            );
            annots[l.from].push(id);
        }
        for (i, page) in self.pages.iter().enumerate() {
            assert!(
                !page.dict.is_empty(),
                "page {i} was reserved but never filled"
            );
            let mut d = page.dict.clone();
            if !annots[i].is_empty() {
                d.push_str(" /Annots [");
                for a in &annots[i] {
                    d.push_str(&format!("{a} 0 R "));
                }
                d.push(']');
            }
            d.push_str(" >>");
            let id = page.id;
            self.objects[id as usize - 1] = Object::Plain(d);
        }
        // Pages tree.
        let kids: Vec<String> = self.pages.iter().map(|p| format!("{} 0 R", p.id)).collect();
        self.set(
            self.pages_id,
            format!(
                "<< /Type /Pages /Kids [{}] /Count {} >>",
                kids.join(" "),
                kids.len()
            ),
        );
        // Page labels: one range per page, each its own prefix.
        let mut nums = String::new();
        for (i, p) in self.pages.iter().enumerate() {
            nums.push_str(&format!("{i} << /P {} >> ", pdf_string(&p.label)));
        }
        // Outlines.
        let outlines_id = self.alloc();
        let items = std::mem::take(&mut self.outline);
        let top_ids: Vec<u32> = items.iter().map(|_| self.alloc()).collect();
        let total = self.write_outlines(&items, outlines_id, &top_ids);
        let mut od = String::from("<< /Type /Outlines");
        if let (Some(f), Some(l)) = (top_ids.first(), top_ids.last()) {
            od.push_str(&format!(" /First {f} 0 R /Last {l} 0 R /Count {total}"));
        }
        od.push_str(" >>");
        self.set(outlines_id, od);
        // Output intent, metadata, info, catalog.
        let icc = self.alloc();
        self.stream(icc, "/N 3", include_bytes!("sRGB-v4.icc").to_vec(), false);
        let intent = self.alloc();
        self.set(
            intent,
            format!(
                "<< /Type /OutputIntent /S /GTS_PDFA1 /OutputConditionIdentifier (sRGB IEC61966-2.1) \
                 /Info (sRGB IEC61966-2.1) /RegistryName (http://www.color.org) /DestOutputProfile {icc} 0 R >>"
            ),
        );
        let meta = self.alloc();
        let xmp = self.xmp();
        self.stream(meta, "/Type /Metadata /Subtype /XML", xmp, true);
        let info = self.alloc();
        let date = pdf_date(&self.info.date);
        self.set(
            info,
            format!(
                "<< /Title {} /Author {} /Creator {} /Producer {} /CreationDate ({date}) /ModDate ({date}) >>",
                pdf_string(&self.info.title),
                pdf_string(&self.info.author),
                pdf_string(&self.info.creator),
                pdf_string(&self.info.creator)
            ),
        );
        let catalog = self.alloc();
        self.set(
            catalog,
            format!(
                "<< /Type /Catalog /Pages {} 0 R /Metadata {meta} 0 R /OutputIntents [{intent} 0 R] \
                 /Outlines {outlines_id} 0 R /PageMode /UseOutlines /PageLabels << /Nums [{nums}] >> \
                 /Lang {} >>",
                self.pages_id,
                pdf_string(&self.info.lang)
            ),
        );
        self.serialise(catalog, info)
    }

    fn xmp(&self) -> Vec<u8> {
        let i = &self.info;
        let t = format!("{}T00:00:00Z", i.date);
        format!(
            "<?xpacket begin=\"\u{FEFF}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\n\
             <x:xmpmeta xmlns:x=\"adobe:ns:meta/\">\n\
             <rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\n\
             <rdf:Description rdf:about=\"\" xmlns:pdfaid=\"http://www.aiim.org/pdfa/ns/id/\" \
             xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:xmp=\"http://ns.adobe.com/xap/1.0/\" \
             xmlns:pdf=\"http://ns.adobe.com/pdf/1.3/\">\n\
             <pdfaid:part>2</pdfaid:part><pdfaid:conformance>B</pdfaid:conformance>\n\
             <dc:title><rdf:Alt><rdf:li xml:lang=\"x-default\">{}</rdf:li></rdf:Alt></dc:title>\n\
             <dc:creator><rdf:Seq><rdf:li>{}</rdf:li></rdf:Seq></dc:creator>\n\
             <xmp:CreateDate>{t}</xmp:CreateDate><xmp:ModifyDate>{t}</xmp:ModifyDate>\n\
             <xmp:CreatorTool>{}</xmp:CreatorTool>\n<pdf:Producer>{}</pdf:Producer>\n\
             </rdf:Description>\n</rdf:RDF>\n</x:xmpmeta>\n<?xpacket end=\"w\"?>",
            xml(&i.title),
            xml(&i.author),
            xml(&i.creator),
            xml(&i.creator)
        )
        .into_bytes()
    }

    /// Lay the objects out: plain ones packed into object streams of a
    /// hundred, streams on their own, a cross-reference stream last.
    fn serialise(self, catalog: u32, info: u32) -> Vec<u8> {
        let mut out: Vec<u8> = Vec::with_capacity(1 << 16);
        out.extend_from_slice(b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n");
        let n_objs = self.objects.len() as u32;
        let plain: Vec<u32> = (1..=n_objs)
            .filter(|&id| matches!(self.objects[id as usize - 1], Object::Plain(_)))
            .collect();
        let n_stm = plain.len().div_ceil(100) as u32;
        let xref_id = n_objs + n_stm + 1;
        // (type, field 2, field 3) per object id, 0 = free.
        let mut rows: Vec<(u8, u64, u16)> = vec![(0, 0, 0xFFFF); xref_id as usize + 1];
        for (k, chunk) in plain.chunks(100).enumerate() {
            let sid = n_objs + 1 + k as u32;
            let mut header = String::new();
            let mut body = Vec::new();
            for (j, &id) in chunk.iter().enumerate() {
                let Object::Plain(s) = &self.objects[id as usize - 1] else {
                    unreachable!()
                };
                header.push_str(&format!("{id} {} ", body.len()));
                body.extend_from_slice(s.as_bytes());
                body.push(b'\n');
                rows[id as usize] = (2, u64::from(sid), j as u16);
            }
            header.push('\n');
            let mut data = header.clone().into_bytes();
            data.extend_from_slice(&body);
            let z = compress_to_vec_zlib(&data, 6);
            rows[sid as usize] = (1, out.len() as u64, 0);
            out.extend_from_slice(
                format!(
                    "{sid} 0 obj\n<< /Type /ObjStm /N {} /First {} /Filter /FlateDecode /Length {} >>\nstream\n",
                    chunk.len(),
                    header.len(),
                    z.len()
                )
                .as_bytes(),
            );
            out.extend_from_slice(&z);
            out.extend_from_slice(b"\nendstream\nendobj\n");
        }
        for (i, o) in self.objects.iter().enumerate() {
            let id = i as u32 + 1;
            match o {
                Object::Stream { dict, data } => {
                    rows[id as usize] = (1, out.len() as u64, 0);
                    out.extend_from_slice(format!("{id} 0 obj\n{dict}\nstream\n").as_bytes());
                    out.extend_from_slice(data);
                    out.extend_from_slice(b"\nendstream\nendobj\n");
                }
                Object::Plain(_) => {}
                Object::Unset => panic!("object {id} was allocated but never written"),
            }
        }
        let xref_off = out.len();
        rows[xref_id as usize] = (1, xref_off as u64, 0);
        let fid = file_id(&out);
        let mut table = Vec::with_capacity(rows.len() * 7);
        for (t, a, b) in &rows {
            table.push(*t);
            table.extend_from_slice(&(*a as u32).to_be_bytes());
            table.extend_from_slice(&b.to_be_bytes());
        }
        let z = compress_to_vec_zlib(&table, 6);
        out.extend_from_slice(
            format!(
                "{xref_id} 0 obj\n<< /Type /XRef /Size {} /W [1 4 2] /Root {catalog} 0 R /Info {info} 0 R \
                 /ID [<{fid}> <{fid}>] /Filter /FlateDecode /Length {} >>\nstream\n",
                rows.len(),
                z.len()
            )
            .as_bytes(),
        );
        out.extend_from_slice(&z);
        out.extend_from_slice(
            format!("\nendstream\nendobj\nstartxref\n{xref_off}\n%%EOF\n").as_bytes(),
        );
        out
    }
}

/// Points from millimetres, two decimals.
fn pt(mm: f64) -> String {
    let v = mm * 72.0 / 25.4;
    let s = format!("{v:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// `D:YYYYMMDD000000Z` from `YYYY-MM-DD`.
fn pdf_date(date: &str) -> String {
    let digits: String = date.chars().filter(char::is_ascii_digit).collect();
    format!("D:{digits:0<8}000000Z")
}

/// A text string: literal when it is plain ASCII, UTF-16BE otherwise.
pub fn pdf_string(s: &str) -> String {
    if s.is_ascii() {
        let mut o = String::from("(");
        for c in s.chars() {
            match c {
                '(' | ')' | '\\' => {
                    o.push('\\');
                    o.push(c);
                }
                '\n' => o.push_str("\\n"),
                '\r' => o.push_str("\\r"),
                c if (c as u32) < 32 => o.push(' '),
                c => o.push(c),
            }
        }
        o.push(')');
        o
    } else {
        let mut o = String::from("<FEFF");
        for u in s.encode_utf16() {
            o.push_str(&format!("{u:04X}"));
        }
        o.push('>');
        o
    }
}

fn xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Sixteen bytes that change whenever the file does (FNV-1a over the
/// body, twice with different seeds), as the trailer's /ID.
fn file_id(bytes: &[u8]) -> String {
    let mut out = String::new();
    for seed in [0xcbf2_9ce4_8422_2325u64, 0x8422_2325_cbf2_9ce4u64] {
        let mut h = seed;
        for b in bytes {
            h ^= u64::from(*b);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
        out.push_str(&format!("{h:016X}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tiny() -> Vec<u8> {
        let mut doc = Doc::new(DocInfo {
            title: "Test".into(),
            author: "Kubuz".into(),
            creator: "test".into(),
            date: "2026-10-28".into(),
            lang: "nl".into(),
        });
        let mut frame = Content::new();
        frame.rect_stroke(20.0, 10.0, 180.0, 277.0);
        doc.add_form("FrA4", 210.0, 297.0, frame);
        let mut c = Content::new();
        c.text([30.0, 30.0], 2.5, 0.0, "Ø9 – é");
        doc.add_page(210.0, 297.0, &["FrA4"], c, "Pos 1");
        doc.add_outline(&["Stuktekeningen"], "Pos 1", 0);
        doc.finish()
    }

    #[test]
    fn writes_a_pdf_with_xref_stream_and_object_streams() {
        let pdf = tiny();
        let s = String::from_utf8_lossy(&pdf);
        assert!(s.starts_with("%PDF-1.7\n"));
        assert!(s.contains("/Type /ObjStm"));
        assert!(s.contains("/Type /XRef"));
        assert!(s.ends_with("%%EOF\n"));
        assert!(s.contains("/Length1 "));
        assert!(s.contains("pdfaid:part>2<"));
        // The startxref offset points at the xref object.
        let off: usize = s
            .rsplit("startxref\n")
            .next()
            .unwrap()
            .lines()
            .next()
            .unwrap()
            .parse()
            .unwrap();
        // Offsets count bytes, and the compressed streams are not UTF-8.
        let tail = String::from_utf8_lossy(&pdf[off..off + 20]).to_string();
        let num = tail.split(' ').next().unwrap();
        assert!(tail.starts_with(&format!("{num} 0 obj")), "{tail}");
    }

    #[test]
    fn strings_escape_or_go_utf16() {
        assert_eq!(pdf_string("a(b)"), "(a\\(b\\))");
        assert_eq!(pdf_string("é"), "<FEFF00E9>");
        assert_eq!(pdf_date("2026-10-28"), "D:20261028000000Z");
        assert_eq!(pt(210.0), "595.28");
    }
}
