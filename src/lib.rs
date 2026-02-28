mod parser;
mod protocol;
mod python;

use pyo3::prelude::*;
use python::bindings::{ThriftParser, BinaryProtocol, ThriftStruct, ThriftField,
                       PyThriftService, PyThriftMethod, ThriftServer, TransportType,
                       ThriftStructInstance};

/// A Python module implemented in Rust.
#[pymodule]
fn thrift_rs_pyo3(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<ThriftParser>()?;
    m.add_class::<BinaryProtocol>()?;
    m.add_class::<ThriftStruct>()?;
    m.add_class::<ThriftField>()?;
    m.add_class::<ThriftStructInstance>()?;
    m.add_class::<PyThriftService>()?;
    m.add_class::<PyThriftMethod>()?;
    m.add_class::<ThriftServer>()?;
    m.add_class::<TransportType>()?;
    Ok(())
}
