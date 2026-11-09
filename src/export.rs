use crate::disk_scanner::{DiskUsage, Kind, Node, NodeId};
use humansize::{format_size, DECIMAL};
use rust_xlsxwriter::{Color, Format, Workbook, XlsxError};
use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::Path;
use tempfile::NamedTempFile;

const HEADERS: [&str; 4] = ["Name", "Type", "Allocated size (bytes)", "Size"];
const EXCEL_ROWS: usize = 1_048_575;
const EXCEL_MAX_EXACT_DECIMAL: u64 = 999_999_999_999_999;

#[derive(Clone, Copy)]
pub enum ExportFormat {
    Excel,
    Csv,
}

impl ExportFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Excel => "xlsx",
            Self::Csv => "csv",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Excel => "Excel workbook (.xlsx)",
            Self::Csv => "CSV file (.csv)",
        }
    }

    pub fn save(
        self,
        usage: &DiskUsage,
        folder: NodeId,
        path: &Path,
        overwrite: bool,
    ) -> io::Result<()> {
        let directory = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let mut file = NamedTempFile::new_in(directory)?;
        let entries = &usage.nodes[usage.children(folder)];
        match self {
            Self::Excel => write_excel(file.as_file_mut(), entries).map_err(io::Error::other)?,
            Self::Csv => write_csv(BufWriter::new(file.as_file_mut()), entries)?,
        }
        file.as_file().sync_all()?;
        let result = if overwrite {
            file.persist(path)
        } else {
            file.persist_noclobber(path)
        };
        result.map(|_| ()).map_err(|error| error.error)
    }
}

fn entry_size(node: &Node) -> Option<u64> {
    match node.kind {
        Kind::Folder | Kind::File => Some(node.size),
        Kind::Unreadable | Kind::OtherVolume => None,
    }
}

fn entry_type(node: &Node) -> &'static str {
    match node.kind {
        Kind::Folder => "Folder",
        Kind::File => "File",
        Kind::Unreadable => "Unavailable",
        Kind::OtherVolume => "Separate volume",
    }
}

fn write_csv(mut out: impl Write, entries: &[Node]) -> io::Result<()> {
    // A UTF-8 BOM lets Excel recognize non-ASCII filenames when opening the file directly.
    out.write_all(b"\xef\xbb\xbf")?;
    writeln!(out, "{}\r", HEADERS.join(","))?;
    for node in entries {
        let name = node.name.to_string_lossy();
        write_csv_name(&mut out, &name)?;
        let kind = entry_type(node);
        if let Some(bytes) = entry_size(node) {
            writeln!(out, ",{kind},{bytes},{}\r", format_size(bytes, DECIMAL))?;
        } else {
            writeln!(out, ",{kind},,\r")?;
        }
    }
    out.flush()
}

fn write_csv_name(out: &mut impl Write, name: &str) -> io::Result<()> {
    out.write_all(b"\"")?;
    let significant = name.trim_start_matches(|ch: char| ch.is_whitespace() || ch.is_control());
    if significant.starts_with(['=', '+', '-', '@', '＝', '＋', '－', '＠'])
        || name.starts_with(['\t', '\r', '\n'])
    {
        // Keep formula-like filenames as text when the CSV is opened in Excel.
        out.write_all(b"\t")?;
    }
    for part in name.split_inclusive('"') {
        out.write_all(part.as_bytes())?;
        if part.ends_with('"') {
            out.write_all(b"\"")?;
        }
    }
    out.write_all(b"\"")
}

fn write_excel(out: &mut File, entries: &[Node]) -> Result<(), XlsxError> {
    let mut workbook = Workbook::new();
    let heading = Format::new()
        .set_bold()
        .set_font_color(Color::White)
        .set_background_color(Color::RGB(0x234DB4));
    let bytes_format = Format::new().set_num_format("#,##0");

    // Each sheet has a header. Split oversized folders without dropping rows.
    for page in 0..entries.len().max(1).div_ceil(EXCEL_ROWS) {
        let start = page * EXCEL_ROWS;
        let nodes = &entries[start..(start + EXCEL_ROWS).min(entries.len())];
        let sheet = workbook.add_worksheet_with_constant_memory()?;
        if page == 0 {
            sheet.set_name("Results")?;
        } else {
            sheet.set_name(format!("Results {}", page + 1))?;
        }
        sheet.set_column_width(0, 50)?;
        sheet.set_column_width(1, 19)?;
        sheet.set_column_width(2, 25)?;
        sheet.set_column_width(3, 17)?;
        sheet.set_freeze_panes(1, 0)?;
        for (column, label) in HEADERS.iter().enumerate() {
            sheet.write_string_with_format(0, column as u16, *label, &heading)?;
        }
        for (index, node) in nodes.iter().enumerate() {
            let row = index as u32 + 1;
            sheet.write_string(row, 0, node.name.to_string_lossy())?;
            sheet.write_string(row, 1, entry_type(node))?;
            if let Some(bytes) = entry_size(node) {
                if bytes <= EXCEL_MAX_EXACT_DECIMAL {
                    sheet.write_number_with_format(row, 2, bytes as f64, &bytes_format)?;
                } else {
                    // Preserve the exact count when it exceeds Excel's numeric precision.
                    sheet.write_string(row, 2, bytes.to_string())?;
                }
                sheet.write_string(row, 3, format_size(bytes, DECIMAL))?;
            }
        }
        sheet.autofilter(0, 0, nodes.len() as u32, HEADERS.len() as u16 - 1)?;
    }
    workbook.save_to_writer(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::disk_scanner::{scan, tests::Fixture, ScanControl};

    #[test]
    fn csv_exports_folder_contents_and_escapes_formula_like_names() {
        let fixture = Fixture::new();
        let bytes = fixture.write("folder/report, draft.txt", 4096);
        fixture.write("folder/ =1+1.txt", 4096);
        fixture.write("folder/subfolder/nested.txt", 4096);
        fixture.write("outside.txt", 4096);
        let usage = scan(&fixture.0, &ScanControl::default()).unwrap();
        let folder = usage
            .children(0)
            .find(|&id| usage.nodes[id].name == "folder")
            .unwrap();
        let output = fixture.0.join("result.csv");
        ExportFormat::Csv
            .save(&usage, folder, &output, false)
            .unwrap();
        let result = std::fs::read_to_string(&output).unwrap();
        assert!(result.starts_with('\u{feff}'));
        assert!(result.contains(&format!("\"report, draft.txt\",File,{bytes},")));
        let mut escaped = Vec::new();
        write_csv_name(&mut escaped, "report, \"draft\".txt").unwrap();
        assert_eq!(escaped, b"\"report, \"\"draft\"\".txt\"");
        assert!(result.contains("\"\t =1+1.txt\",File,"));
        assert!(result.contains("\"subfolder\",Folder,"));
        assert!(!result.contains("outside.txt"));
        assert!(!result.contains("nested.txt"));
        assert!(ExportFormat::Csv.save(&usage, 0, &output, false).is_err());
        assert_eq!(std::fs::read_to_string(&output).unwrap(), result);
    }

    #[test]
    fn excel_exports_a_workbook_even_for_an_empty_folder() {
        let fixture = Fixture::new();
        let usage = scan(&fixture.0, &ScanControl::default()).unwrap();
        let output = fixture.0.join("result.xlsx");
        ExportFormat::Excel.save(&usage, 0, &output, false).unwrap();
        assert!(std::fs::read(output).unwrap().starts_with(b"PK\x03\x04"));
    }
}
