//! Real-valued MATLAB-style numerical engine. The UI is an optional consumer.
/// MATLAB on x86 produces the negative quiet NaN (0xfff8000000000000).
pub const MATLAB_NAN: f64 = f64::from_bits(0xfff8_0000_0000_0000);

pub mod array;
pub mod bench;
mod broadcast2;
pub mod categorical_array;
mod clock;
pub mod complex_matrix;
pub mod containers;
pub mod datetime_array;
pub mod display_format;
pub mod document;
pub mod dsp;
pub mod duration_array;
pub mod editor;
pub mod engine;
#[cfg(feature = "plot-export")]
pub mod export;
pub mod files;
pub mod function_handle;
pub mod help;
pub mod logical;
pub mod matrix;
mod parallel;
pub mod plot;
pub mod scalar;
pub mod script_containers;
pub mod session;
pub mod simulation;
pub mod sparse;
pub mod string_array;
pub mod struct_array;
pub use string_array::StringArray;
mod struct_display;
pub mod syntax;
pub mod table_array;
pub mod typed_array;
pub mod variable_edit;

pub use array::Array;
pub use categorical_array::CategoricalArray;
pub use complex_matrix::{Complex64, ComplexMatrix};
pub use containers::CellArray;
pub use datetime_array::DatetimeArray;
pub use duration_array::DurationArray;
pub use engine::{Engine, Execution, HostIo};
pub use function_handle::FunctionHandle;
pub use logical::LogicalMatrix;
pub use matrix::Matrix;
pub use parallel::initialize_worker_pool;
pub use scalar::Scalar;
pub use script_containers::CharacterRow;
pub use struct_array::StructArray;
pub use table_array::TableArray;
pub use typed_array::{NumericClass, TypedArray};
pub type Result<T> = std::result::Result<T, String>;

/// Release version shown by the console, the desktop banner and About.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub mod graphics_objects;
pub use graphics_objects::{GraphicsArray, GraphicsHandle, GraphicsKind, ObjectIdentity};
