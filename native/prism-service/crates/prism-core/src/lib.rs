//! 棱镜（Prism）的公共类型。
//!
//! 这里只放各 crate 都要用到的类型和默认值，不做任何 IO。
//! 状态流转规则、错误分类规则见 `docs/detail-design.md` 第 1、2 章。

// 宏要先于使用它的模块声明
#[macro_use]
pub mod parse;

pub mod defaults;
pub mod error;
pub mod fission;
pub mod ids;
pub mod status;

pub use error::{ApiError, ApiErrorCode, RetryPolicy, UpstreamErrorCode};
pub use fission::{FissionMode, Lock, Strength, VariantAxis};
pub use ids::{new_id, now_ms};
pub use parse::ParseEnumError;
pub use status::{BatchStatus, JobKind, JobStatus};
