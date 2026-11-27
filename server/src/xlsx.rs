//! Minimal Excel (.xlsx) writer: SpreadsheetML in an uncompressed ZIP.
//! No dependencies and a few KB of code; the HTTP layer gzips the response.
//! Supports text and number cells, a bold frozen header row with filters,
//! column widths and several sheets.

pub enum Cell {
    Text(String),
    Num(f64),
    Empty,
}

impl From<&str> for Cell {
    fn from(s: &str) -> Self {
        if s.is_empty() { Cell::Empty } else { Cell::Text(s.to_string()) }
    }
}
impl From<String> for Cell {
    fn from(s: String) -> Self {
        if s.is_empty() { Cell::Empty } else { Cell::Text(s) }
    }
}
impl From<f64> for Cell {
    fn from(v: f64) -> Self {
        if v.is_finite() { Cell::Num(v) } else { Cell::Empty }
    }
}
impl From<i64> for Cell {
    fn from(v: i64) -> Self {
        Cell::Num(v as f64)
    }
}
impl From<i32> for Cell {
    fn from(v: i32) -> Self {
        Cell::Num(v as f64)
    }
}
impl<T: Into<Cell>> From<Option<T>> for Cell {
    fn from(v: Option<T>) -> Self {
        v.map(Into::into).unwrap_or(Cell::Empty)
    }
}

/// One table: used for both the CSV and the Excel export.
pub struct Sheet {
    pub name: String,
    pub headers: Vec<String>,
    pub rows: Vec<Vec<Cell>>,
}

impl Sheet {
    pub fn new(name: &str, headers: &[&str]) -> Self {
        Self { name: name.into(), headers: headers.iter().map(|h| h.to_string()).collect(), rows: Vec::new() }
    }

    pub fn push(&mut self, row: Vec<Cell>) {
        self.rows.push(row);
    }

    /// CSV (with formula-injection guard on text cells).
    pub fn to_csv(&self) -> String {
        let mut s = self.headers.iter().map(|h| crate::util::csv_field(h)).collect::<Vec<_>>().join(",");
        s.push('\n');
        for r in &self.rows {
            let line: Vec<String> = r
                .iter()
                .map(|c| match c {
                    Cell::Text(t) => crate::util::csv_field(t),
                    Cell::Num(n) => num(*n),
                    Cell::Empty => String::new(),
                })
                .collect();
            s.push_str(&line.join(","));
            s.push('\n');
        }
        s
    }
}

fn num(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e15 { format!("{}", n as i64) } else { format!("{:.2}", n) }
}

fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            // control characters are not allowed in XML 1.0
            c if (c as u32) < 0x20 && !matches!(c, '\t' | '\n' | '\r') => {}
            c => o.push(c),
        }
    }
    o
}

fn col_name(mut i: usize) -> String {
    let mut s = Vec::new();
    loop {
        s.push(b'A' + (i % 26) as u8);
        if i < 26 {
            break;
        }
        i = i / 26 - 1;
    }
    s.reverse();
    String::from_utf8(s).unwrap()
}

fn sheet_name(n: &str, i: usize) -> String {
    let clean: String = n.chars().filter(|c| !matches!(c, '[' | ']' | ':' | '*' | '?' | '/' | '\\')).take(31).collect();
    if clean.trim().is_empty() { format!("Sheet{}", i + 1) } else { clean }
}

fn sheet_xml(sh: &Sheet) -> String {
    let ncol = sh.headers.len().max(sh.rows.iter().map(|r| r.len()).max().unwrap_or(0)).max(1);
    // rough column widths from content (Thai characters count about one width unit each)
    let mut widths = vec![8usize; ncol];
    for (i, h) in sh.headers.iter().enumerate() {
        widths[i] = widths[i].max(h.chars().count() + 2);
    }
    for r in sh.rows.iter().take(500) {
        for (i, c) in r.iter().enumerate() {
            let w = match c {
                Cell::Text(t) => t.chars().count(),
                Cell::Num(n) => num(*n).len(),
                Cell::Empty => 0,
            };
            widths[i] = widths[i].max(w + 2);
        }
    }
    let mut x = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetViews><sheetView workbookViewId="0"><pane ySplit="1" topLeftCell="A2" activePane="bottomLeft" state="frozen"/></sheetView></sheetViews><cols>"#,
    );
    for (i, w) in widths.iter().enumerate() {
        x.push_str(&format!(r#"<col min="{0}" max="{0}" width="{1}" customWidth="1"/>"#, i + 1, (*w).min(60)));
    }
    x.push_str("</cols><sheetData>");
    x.push_str(r#"<row r="1">"#);
    for (i, h) in sh.headers.iter().enumerate() {
        x.push_str(&format!(r#"<c r="{}1" t="inlineStr" s="1"><is><t>{}</t></is></c>"#, col_name(i), esc(h)));
    }
    x.push_str("</row>");
    for (ri, r) in sh.rows.iter().enumerate() {
        let rn = ri + 2;
        x.push_str(&format!(r#"<row r="{rn}">"#));
        for (ci, c) in r.iter().enumerate() {
            match c {
                Cell::Text(t) => x.push_str(&format!(r#"<c r="{}{rn}" t="inlineStr"><is><t xml:space="preserve">{}</t></is></c>"#, col_name(ci), esc(t))),
                Cell::Num(n) => x.push_str(&format!(r#"<c r="{}{rn}" s="{}"><v>{}</v></c>"#, col_name(ci), if n.fract() == 0.0 { 0 } else { 2 }, n)),
                Cell::Empty => {}
            }
        }
        x.push_str("</row>");
    }
    x.push_str("</sheetData>");
    if !sh.headers.is_empty() {
        x.push_str(&format!(r#"<autoFilter ref="A1:{}{}"/>"#, col_name(ncol - 1), sh.rows.len() + 1));
    }
    x.push_str("</worksheet>");
    x
}

/// Builds the .xlsx file.
pub fn workbook(sheets: &[Sheet]) -> Vec<u8> {
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    let mut ct = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/>"#,
    );
    let mut wb = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets>"#,
    );
    let mut rels = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#,
    );
    let mut used = std::collections::HashSet::new();
    for (i, sh) in sheets.iter().enumerate() {
        let n = i + 1;
        let mut name = sheet_name(&sh.name, i);
        while !used.insert(name.to_lowercase()) {
            name = format!("{}{}", name.chars().take(28).collect::<String>(), n);
        }
        ct.push_str(&format!(r#"<Override PartName="/xl/worksheets/sheet{n}.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>"#));
        wb.push_str(&format!(r#"<sheet name="{}" sheetId="{n}" r:id="rId{n}"/>"#, esc(&name)));
        rels.push_str(&format!(r#"<Relationship Id="rId{n}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet{n}.xml"/>"#));
        files.push((format!("xl/worksheets/sheet{n}.xml"), sheet_xml(sh).into_bytes()));
    }
    // a hidden filter range name is not required; Excel accepts autoFilter alone
    ct.push_str("</Types>");
    wb.push_str("</sheets></workbook>");
    rels.push_str(&format!(
        r#"<Relationship Id="rId{}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/></Relationships>"#,
        sheets.len() + 1
    ));
    let styles = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><fonts count="2"><font><sz val="11"/><name val="Tahoma"/></font><font><b/><sz val="11"/><name val="Tahoma"/></font></fonts><fills count="3"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill><fill><patternFill patternType="solid"><fgColor rgb="FFDDEBF7"/></patternFill></fill></fills><borders count="1"><border/></borders><cellStyleXfs count="1"><xf/></cellStyleXfs><cellXfs count="3"><xf/><xf fontId="1" fillId="2" applyFont="1" applyFill="1"/><xf numFmtId="2" applyNumberFormat="1"/></cellXfs><cellStyles count="1"><cellStyle name="Normal" xfId="0" builtinId="0"/></cellStyles></styleSheet>"#;
    let root_rels = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#;
    let mut all: Vec<(String, Vec<u8>)> = vec![
        ("[Content_Types].xml".into(), ct.into_bytes()),
        ("_rels/.rels".into(), root_rels.as_bytes().to_vec()),
        ("xl/workbook.xml".into(), wb.into_bytes()),
        ("xl/_rels/workbook.xml.rels".into(), rels.into_bytes()),
        ("xl/styles.xml".into(), styles.as_bytes().to_vec()),
    ];
    all.extend(files);
    zip_store(&all)
}

fn crc32(data: &[u8]) -> u32 {
    static TABLE: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
    let t = TABLE.get_or_init(|| {
        let mut t = [0u32; 256];
        for (i, e) in t.iter_mut().enumerate() {
            let mut c = i as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            }
            *e = c;
        }
        t
    });
    let mut c = 0xFFFF_FFFFu32;
    for b in data {
        c = t[((c ^ *b as u32) & 0xFF) as usize] ^ (c >> 8);
    }
    c ^ 0xFFFF_FFFF
}

/// ZIP archive with "stored" (uncompressed) entries.
fn zip_store(files: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, data) in files {
        let crc = crc32(data);
        let off = out.len() as u32;
        let n = name.as_bytes();
        let le16 = |v: u16| v.to_le_bytes();
        let le32 = |v: u32| v.to_le_bytes();
        // local file header
        out.extend_from_slice(&le32(0x0403_4b50));
        out.extend_from_slice(&le16(20));
        out.extend_from_slice(&le16(0x0800)); // UTF-8 names
        out.extend_from_slice(&le16(0)); // stored
        out.extend_from_slice(&le16(0));
        out.extend_from_slice(&le16(0x21)); // 1980-01-01
        out.extend_from_slice(&le32(crc));
        out.extend_from_slice(&le32(data.len() as u32));
        out.extend_from_slice(&le32(data.len() as u32));
        out.extend_from_slice(&le16(n.len() as u16));
        out.extend_from_slice(&le16(0));
        out.extend_from_slice(n);
        out.extend_from_slice(data);
        // central directory entry
        central.extend_from_slice(&le32(0x0201_4b50));
        central.extend_from_slice(&le16(20));
        central.extend_from_slice(&le16(20));
        central.extend_from_slice(&le16(0x0800));
        central.extend_from_slice(&le16(0));
        central.extend_from_slice(&le16(0));
        central.extend_from_slice(&le16(0x21));
        central.extend_from_slice(&le32(crc));
        central.extend_from_slice(&le32(data.len() as u32));
        central.extend_from_slice(&le32(data.len() as u32));
        central.extend_from_slice(&le16(n.len() as u16));
        central.extend_from_slice(&[0u8; 8]); // extra len, comment len, disk start, internal attrs
        central.extend_from_slice(&le32(0)); // external attrs
        central.extend_from_slice(&le32(off));
        central.extend_from_slice(n);
    }
    let cd_off = out.len() as u32;
    let cd_len = central.len() as u32;
    out.extend_from_slice(&central);
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&[0u8; 4]);
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&cd_len.to_le_bytes());
    out.extend_from_slice(&cd_off.to_le_bytes());
    out.extend_from_slice(&[0u8; 2]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_and_columns() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(col_name(0), "A");
        assert_eq!(col_name(25), "Z");
        assert_eq!(col_name(26), "AA");
        assert_eq!(col_name(701), "ZZ");
    }

    #[test]
    fn csv_guard() {
        let mut s = Sheet::new("t", &["a", "b"]);
        s.push(vec!["=cmd()".into(), 1.5f64.into()]);
        assert!(s.to_csv().contains("'=cmd()"));
        let wb = workbook(&[s]);
        assert!(wb.starts_with(b"PK"));
        // end-of-central-directory points at a real central directory header
        let eocd = wb.len() - 22;
        let cd_off = u32::from_le_bytes(wb[eocd + 16..eocd + 20].try_into().unwrap()) as usize;
        assert_eq!(&wb[cd_off..cd_off + 4], &[0x50, 0x4b, 0x01, 0x02]);
    }
}
