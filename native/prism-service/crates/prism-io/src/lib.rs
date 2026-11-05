//! 表格读写（详细设计第 11 章）：读 Excel / CSV、列映射、按导出模板写 xlsx、ZIP 打包和文件命名。
//!
//! 不知道表格是哪个平台的，也不碰数据库：调用方给字段和数据，这里只按规则和模板执行。

mod csv_read;
mod error;
mod export;
mod mapping;
mod naming;
mod pack;
mod table;

pub use error::IoError;
pub use export::{ExportProblem, ExportReport, ExportValue, RowData, write_xlsx};
pub use mapping::{
    FieldSpec, MappedRow, MappedTable, Mapping, RowProblem, SAVED_MAPPING_MIN_OVERLAP,
    apply_mapping, auto_map, header_overlap,
};
pub use naming::{render_name, sanitize_file_name};
pub use pack::{ZipEntry, ZipReport, ZipSource, write_zip};
pub use table::{
    CellValue, MAX_ROWS, MAX_TABLE_BYTES, ReadOptions, Table, TableRow, read_table,
    read_table_bytes,
};

/// 表格读写的返回类型。
pub type Result<T> = std::result::Result<T, IoError>;
