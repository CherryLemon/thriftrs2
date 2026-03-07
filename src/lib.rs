mod parser;
mod protocol;
mod python;

use pyo3::prelude::*;
use python::client::{ThriftApplicationException, ThriftClient};
use python::parser::{BinaryProtocol, CompactProtocol, JSONProtocol, ProtocolType, ThriftParser};
use python::server::ThriftServer;
use python::types::{
    PyThriftMethod, PyThriftService, ThriftField, ThriftStruct, ThriftStructInstance, TransportType,
};

/// A Python module implemented in Rust.
#[pymodule]
fn thriftrs2(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<ThriftParser>()?;
    m.add_class::<BinaryProtocol>()?;
    m.add_class::<ThriftStruct>()?;
    m.add_class::<ThriftField>()?;
    m.add_class::<ThriftStructInstance>()?;
    m.add_class::<PyThriftService>()?;
    m.add_class::<PyThriftMethod>()?;
    m.add_class::<ThriftServer>()?;
    m.add_class::<ProtocolType>()?;
    m.add_class::<CompactProtocol>()?;
    m.add_class::<JSONProtocol>()?;
    m.add_class::<TransportType>()?;
    m.add_class::<ThriftClient>()?;
    m.add_class::<ThriftApplicationException>()?;
    Ok(())
}
