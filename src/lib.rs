mod parser;
mod protocol;
mod python;

use pyo3::prelude::*;
use python::bindings::{ThriftParser, BinaryProtocol, ThriftStruct, ThriftField};

/// A Python module implemented in Rust.
#[pymodule]
fn thrift_rs_pyo3(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<ThriftParser>()?;
    m.add_class::<BinaryProtocol>()?;
    m.add_class::<ThriftStruct>()?;
    m.add_class::<ThriftField>()?;
    Ok(())
}
