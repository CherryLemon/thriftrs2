// ──────────────────────────────────────────────────────────────────────────────
// server.rs  –  ThriftServer and connection-handling logic (tokio async)
// ──────────────────────────────────────────────────────────────────────────────
use crate::protocol::{
    BinaryProtocolReader, BinaryProtocolWriter, FieldBegin, MessageBegin, TInputProtocol,
    TOutputProtocol, TType, MESSAGE_TYPE_EXCEPTION, MESSAGE_TYPE_ONEWAY,
};
use pyo3::prelude::*;
use pyo3::types::PyDict;
use std::cell::RefCell;
use std::collections::HashMap;
use std::io::Cursor;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Notify;

use super::parser::ThriftParser;
use super::serde::{deserialize_rust_struct, thrift_type_to_ttype, write_value_with_structs};
use super::types::{
    PyThriftService, ThriftField, ThriftStruct, ThriftStructInstance,
    TransportType,
};

thread_local! {
    static WORKER_PY_ASYNCIO: RefCell<Option<WorkerPythonAsyncio>> = const { RefCell::new(None) };
}

struct WorkerPythonAsyncio {
    locals: pyo3_async_runtimes::TaskLocals,
    loop_thread: Option<JoinHandle<()>>,
}

impl WorkerPythonAsyncio {
    fn start() -> Result<Self, String> {
        let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
        let loop_thread = std::thread::Builder::new()
            .name("thrift-rs-pyo3-python-loop".to_string())
            .spawn(move || {
                let locals = match Python::attach(|py| -> PyResult<pyo3_async_runtimes::TaskLocals> {
                    let asyncio = py.import("asyncio")?;
                    let event_loop = asyncio.call_method0("new_event_loop")?;
                    asyncio.call_method1("set_event_loop", (event_loop.clone(),))?;
                    pyo3_async_runtimes::TaskLocals::new(event_loop).copy_context(py)
                }) {
                    Ok(locals) => locals,
                    Err(err) => {
                        let _ = ready_tx.send(Err(err.to_string()));
                        return;
                    }
                };

                if ready_tx.send(Ok(locals.clone())).is_err() {
                    return;
                }

                Python::attach(|py| {
                    let event_loop = locals.event_loop(py);
                    if let Err(err) = event_loop.call_method0("run_forever") {
                        err.print_and_set_sys_last_vars(py);
                    }
                    if let Err(err) = shutdown_python_event_loop(&event_loop) {
                        err.print_and_set_sys_last_vars(py);
                    }
                    if let Ok(asyncio) = py.import("asyncio") {
                        let _ = asyncio.call_method1("set_event_loop", (py.None(),));
                    }
                });
            })
            .map_err(|err| format!("spawn python loop thread: {err}"))?;

        let locals = ready_rx
            .recv()
            .map_err(|_| "python loop thread exited before initialization".to_string())?;
        let locals = locals?;

        Ok(Self {
            locals,
            loop_thread: Some(loop_thread),
        })
    }

    fn stop(mut self) -> Result<(), String> {
        Python::attach(|py| -> PyResult<()> {
            let event_loop = self.locals.event_loop(py);
            if !event_loop.call_method0("is_closed")?.extract::<bool>()? {
                let stop = event_loop.getattr("stop")?;
                event_loop.call_method1("call_soon_threadsafe", (stop,))?;
            }
            Ok(())
        })
        .map_err(|err| err.to_string())?;

        if let Some(handle) = self.loop_thread.take() {
            handle
                .join()
                .map_err(format_thread_panic_payload)?;
        }

        Ok(())
    }
}

fn ensure_current_worker_python_asyncio() -> Result<(), String> {
    WORKER_PY_ASYNCIO.with(|slot| {
        if slot.borrow().is_some() {
            return Ok(());
        }

        let binding = WorkerPythonAsyncio::start()?;
        *slot.borrow_mut() = Some(binding);
        Ok(())
    })
}

fn current_worker_python_locals() -> Result<pyo3_async_runtimes::TaskLocals, String> {
    ensure_current_worker_python_asyncio()?;
    WORKER_PY_ASYNCIO.with(|slot| {
        slot.borrow()
            .as_ref()
            .map(|binding| binding.locals.clone())
            .ok_or_else(|| {
                "python asyncio loop is not bound to the current tokio worker thread".to_string()
            })
    })
}

fn shutdown_current_worker_python_asyncio() -> Result<(), String> {
    WORKER_PY_ASYNCIO.with(|slot| match slot.borrow_mut().take() {
        Some(binding) => binding.stop(),
        None => Ok(()),
    })
}

fn shutdown_python_event_loop(event_loop: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = event_loop.py();
    if event_loop.call_method0("is_closed")?.extract::<bool>()? {
        return Ok(());
    }

    event_loop.call_method1(
        "run_until_complete",
        (event_loop.call_method0("shutdown_asyncgens")?,),
    )?;

    if event_loop.hasattr("shutdown_default_executor")? {
        event_loop.call_method1(
            "run_until_complete",
            (event_loop.call_method0("shutdown_default_executor")?,),
        )?;
    }

    event_loop.call_method0("close")?;
    let _ = py;
    Ok(())
}

fn format_thread_panic_payload(payload: Box<dyn std::any::Any + Send + 'static>) -> String {
    if let Some(msg) = payload.downcast_ref::<String>() {
        msg.clone()
    } else if let Some(msg) = payload.downcast_ref::<&str>() {
        (*msg).to_string()
    } else {
        "python loop thread panicked".to_string()
    }
}

#[pyclass]
pub struct ThriftServer {
    service: PyThriftService,
    handlers: HashMap<String, Py<PyAny>>,
    struct_map: Arc<HashMap<String, ThriftStruct>>,
    transport: TransportType,
    workers: usize,
    // Indicates whether the server is currently running (serve/serve_nonblocking).
    running: Arc<AtomicBool>,
    // Notify used to signal a graceful shutdown to the accept loop/runtime.
    shutdown: Arc<Notify>,
}

#[pymethods]
impl ThriftServer {
    #[new]
    #[pyo3(signature = (service, transport = TransportType::Framed, workers = 1))]
    pub fn new(service: PyThriftService, transport: TransportType, workers: usize) -> Self {
        Self {
            service,
            handlers: HashMap::new(),
            struct_map: Arc::new(HashMap::new()),
            transport,
            workers,
            running: Arc::new(AtomicBool::new(false)),
            shutdown: Arc::new(Notify::new()),
        }
    }

    #[getter]
    pub fn transport(&self) -> TransportType {
        self.transport
    }

    #[setter]
    pub fn set_transport(&mut self, transport: TransportType) {
        self.transport = transport;
    }

    #[getter]
    pub fn workers(&self) -> usize {
        self.workers
    }

    #[setter]
    pub fn set_workers(&mut self, workers: usize) {
        self.workers = workers;
    }

    pub fn set_parser(&mut self, parser: &ThriftParser) {
        self.struct_map = parser.struct_map();
    }

    pub fn register_handler(&mut self, method_name: &str, handler: Py<PyAny>) {
        self.handlers.insert(method_name.to_string(), handler);
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    pub fn serve(&self, py: Python<'_>, host: &str, port: u16) -> PyResult<()> {
        // Prevent starting multiple servers concurrently.
        if self.running.load(Ordering::SeqCst) {
            return Err(PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(
                "server already running",
            ));
        }
        let addr = format!("{}:{}", host, port);
        let service = Arc::new(self.service.clone());
        let handlers: Arc<HashMap<String, Py<PyAny>>> = Arc::new(
            self.handlers
                .iter()
                .map(|(k, v)| (k.clone(), v.clone_ref(py)))
                .collect(),
        );
        let struct_map = Arc::clone(&self.struct_map);
        let transport = self.transport;
        let n_workers = if self.workers == 0 {
            num_cpus::get().max(2)
        } else {
            self.workers
        };

        let rt = build_runtime(n_workers).map_err(|e| {
            PyErr::new::<pyo3::exceptions::PyOSError, _>(format!("tokio runtime: {}", e))
        })?;

         // Mark as running and pass a clone into the accept loop so it can
         // clear the flag if the loop exits.
         let running = Arc::clone(&self.running);
         let shutdown = Arc::clone(&self.shutdown);

         println!(
             "ThriftServer ({:?}, {} workers) listening on {}",
             transport, n_workers, addr
         );

        std::thread::spawn(move || {
             rt.block_on(accept_loop(addr, service, handlers, struct_map, transport, running, shutdown));
        });

        Ok(())
    }

     /// Stop the running server (if any). This will notify the accept loop to exit
     /// and clear the `running` flag.
     pub fn stop(&self) {
         // Clear the running flag and notify the runtime to shutdown the accept loop.
         self.running.store(false, Ordering::SeqCst);
         self.shutdown.notify_waiters();
     }
 }

 // ──────────────────────────────────────────────────────────────────────────────
// Runtime builder
// ──────────────────────────────────────────────────────────────────────────────

fn build_runtime(n_workers: usize) -> std::io::Result<tokio::runtime::Runtime> {
    let mut builder = tokio::runtime::Builder::new_multi_thread();
    builder
        .worker_threads(n_workers)
        .enable_all()
        .on_thread_start(|| {
            if let Err(err) = ensure_current_worker_python_asyncio() {
                eprintln!("python asyncio worker init error: {err}");
            }
        })
        .on_thread_stop(|| {
            if let Err(err) = shutdown_current_worker_python_asyncio() {
                eprintln!("python asyncio worker shutdown error: {err}");
            }
        });
    builder.build()
}

// ──────────────────────────────────────────────────────────────────────────────
// Async accept loop
// ──────────────────────────────────────────────────────────────────────────────

async fn accept_loop(
     addr: String,
     service: Arc<PyThriftService>,
     handlers: Arc<HashMap<String, Py<PyAny>>>,
     struct_map: Arc<HashMap<String, ThriftStruct>>,
     transport: TransportType,
     running: Arc<AtomicBool>,
     shutdown: Arc<Notify>,
 ) {
     let listener = match TcpListener::bind(&addr).await {
         Ok(l) => l,
         Err(e) => {
             eprintln!("bind error on {}: {}", addr, e);
             return;
         }
     };
     running.store(true, Ordering::SeqCst);
     loop {
         // Listen for either a shutdown signal (programmatic) or Ctrl+C, or an incoming connection.
         tokio::select! {
             _ = shutdown.notified() => {
                 println!("shutdown requested");
                 break;
             }
             _ = tokio::signal::ctrl_c() => {
                 println!("received Ctrl+C, shutting down");
                 break;
             }
             accept_res = listener.accept() => {
                 let (stream, _peer) = match accept_res {
                     Ok(v) => v,
                     Err(e) => {
                         eprintln!("accept error: {}", e);
                         continue;
                     }
                 };
 
                 let service = Arc::clone(&service);
                 let handlers = Arc::clone(&handlers);
                 let struct_map = Arc::clone(&struct_map);
 
                 tokio::spawn(async move {
                     if let Err(e) =
                         handle_connection(stream, service, handlers, struct_map, transport).await
                     {
                         use std::io::ErrorKind::*;
                         if e.kind() != UnexpectedEof
                             && e.kind() != ConnectionReset
                             && e.kind() != BrokenPipe
                         {
                             eprintln!("connection error: {}", e);
                         }
                     }
                 });
             }
         }
     }
 
     // If we ever break out of the accept loop, clear the running flag.
     // (In normal operation the loop is infinite; this ensures correctness
     // if the loop ever ends.)
     // Note: unreachable in the current design, but kept for completeness.
     running.store(false, Ordering::SeqCst);
 }

 // ──────────────────────────────────────────────────────────────────────────────
// Async connection handler
// ──────────────────────────────────────────────────────────────────────────────

async fn handle_connection(
    stream: TcpStream,
    service: Arc<PyThriftService>,
    handlers: Arc<HashMap<String, Py<PyAny>>>,
    struct_map: Arc<HashMap<String, ThriftStruct>>,
    transport: TransportType,
) -> std::io::Result<()> {
    let _ = stream.set_nodelay(true);
    let (read_half, write_half) = stream.into_split();
    let mut buf_reader = BufReader::with_capacity(65536, read_half);
    let mut buf_writer = BufWriter::with_capacity(65536, write_half);

    loop {
        let frame: Vec<u8> = match transport {
            TransportType::Framed => {
                let frame_len = match buf_reader.read_i32().await {
                    Ok(n) if n > 0 => n as usize,
                    Ok(_) => return Ok(()),
                    Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(()),
                    Err(e) => return Err(e),
                };
                let mut buf = vec![0u8; frame_len];
                buf_reader.read_exact(&mut buf).await?;
                buf
            }
            TransportType::Buffered => {
                let mut hdr = [0u8; 4];
                match buf_reader.read_exact(&mut hdr).await {
                    Ok(_) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(()),
                    Err(e) => return Err(e),
                }
                let mut name_len_bytes = [0u8; 4];
                buf_reader.read_exact(&mut name_len_bytes).await?;
                let name_len = i32::from_be_bytes(name_len_bytes) as usize;
                let mut name_buf = vec![0u8; name_len];
                buf_reader.read_exact(&mut name_buf).await?;
                let mut seq_id_bytes = [0u8; 4];
                buf_reader.read_exact(&mut seq_id_bytes).await?;

                let mut body: Vec<u8> = Vec::with_capacity(256);
                read_buffered_struct_body_async(&mut buf_reader, &mut body).await?;

                let mut frame =
                    Vec::with_capacity(4 + 4 + name_len + 4 + body.len());
                frame.extend_from_slice(&hdr);
                frame.extend_from_slice(&name_len_bytes);
                frame.extend_from_slice(&name_buf);
                frame.extend_from_slice(&seq_id_bytes);
                frame.extend_from_slice(&body);
                frame
            }
        };

        let mut cursor = Cursor::new(&frame[..]);
        let mut reader = BinaryProtocolReader::new(&mut cursor);
        let msg_begin = reader
            .read_message_begin()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;

        let method_def = service.methods.iter().find(|m| m.name == msg_begin.name);

        let response_payload = match method_def {
            None => build_exception_reply(
                &msg_begin.name,
                msg_begin.seq_id,
                1,
                &format!("Unknown method: {}", msg_begin.name),
            ),
            Some(method) => {
                let args_rust = deserialize_rust_struct(
                    &mut reader,
                    &method.arguments,
                    &method.arg_field_map,
                    &struct_map,
                )
                .map_err(|e| {
                    std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string())
                })?;

                let args_struct_name = format!("{}_args", msg_begin.name);
                let args_field_names = args_rust.field_names;
                let args_values = args_rust.values;

                if !handlers.contains_key(&msg_begin.name) {
                    let payload = build_exception_reply(
                        &msg_begin.name,
                        msg_begin.seq_id,
                        1,
                        &format!("No handler registered for: {}", msg_begin.name),
                    );
                    send_response_async(&mut buf_writer, &payload, transport).await?;
                    continue;
                }

                // Clone what we need to move into closures.
                let method = method.clone();
                let struct_map2 = Arc::clone(&struct_map);
                let handlers2 = Arc::clone(&handlers);
                let method_name = msg_begin.name.clone();
                let seq_id = msg_begin.seq_id;

                // Detect whether the handler is a coroutine function (async def)
                // and build kwargs — all done while holding the GIL briefly.
                enum CallOutcome {
                    /// Sync handler returned a value directly.
                    Sync(Vec<u8>),
                    /// Async handler; yields a coroutine to drive via tokio.
                    Async(Py<PyAny>),
                }

                let outcome: Result<CallOutcome, String> =
                    tokio::task::spawn_blocking({
                        let method = method.clone();
                        let struct_map2 = Arc::clone(&struct_map2);
                        let handlers2 = Arc::clone(&handlers2);
                        let method_name = method_name.clone();
                        let args_struct_name = args_struct_name.clone();
                        let args_field_names = args_field_names.clone();
                        let args_values = args_values.clone();
                        move || {
                            Python::attach(|py| {
                                let handler = handlers2
                                    .get(&method_name)
                                    .expect("handler checked above")
                                    .clone_ref(py);

                                // Detect async def *before* calling.
                                let is_async_fn: bool = py
                                    .import("inspect")?
                                    .call_method1(
                                        "iscoroutinefunction",
                                        (handler.bind(py),),
                                    )?
                                    .extract()?;

                                let schema: Arc<HashMap<String, ThriftField>> =
                                    Arc::new(
                                        method
                                            .arguments
                                            .iter()
                                            .map(|f| (f.name.clone(), f.clone()))
                                            .collect(),
                                    );
                                let args_instance = ThriftStructInstance::from_rust(
                                    args_struct_name,
                                    Arc::new(args_field_names),
                                    args_values,
                                    schema,
                                    Arc::clone(&struct_map2),
                                );
                                let py_args = Bound::new(py, args_instance)?;

                                let kwargs = PyDict::new(py);
                                let mut inst_borrow = py_args.borrow_mut();
                                let field_names: Vec<String> =
                                    inst_borrow.field_names.as_ref().clone();
                                for name in &field_names {
                                    let val = inst_borrow
                                        .get_field(py, name)
                                        .map(|v| v.unbind())
                                        .unwrap_or_else(|| py.None());
                                    kwargs.set_item(name, val)?;
                                }
                                drop(inst_borrow);

                                let call_result =
                                    handler.call(py, (), Some(&kwargs))?;

                                if is_async_fn {
                                    // Return the coroutine object; we'll drive it
                                    // natively on the tokio task via pyo3-async-runtimes.
                                    Ok::<CallOutcome, PyErr>(CallOutcome::Async(
                                        call_result,
                                    ))
                                } else {
                                    // Sync handler: build reply immediately.
                                    let reply_body = build_reply_body(
                                        py,
                                        &method.return_type,
                                        call_result.bind(py),
                                        &struct_map2,
                                    )?;
                                    Ok(CallOutcome::Sync(build_reply_frame(
                                        &method_name,
                                        seq_id,
                                        &reply_body,
                                    )))
                                }
                            })
                            .map_err(|e: PyErr| e.to_string())
                        }
                    })
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()));

                let result: Result<Vec<u8>, String> = match outcome {
                    Err(e) => Err(e),
                    Ok(CallOutcome::Sync(frame)) => Ok(frame),
                    Ok(CallOutcome::Async(coro)) => match current_worker_python_locals() {
                        Err(err) => Err(err),
                        Ok(locals) => {
                            let py_future: Result<_, String> = Python::attach(|py| {
                                pyo3_async_runtimes::into_future_with_locals(
                                    &locals,
                                    coro.into_bound(py),
                                )
                                .map_err(|e: PyErr| e.to_string())
                            });
                            match py_future {
                                Err(e) => Err(e),
                                Ok(fut) => {
                                    let py_result = fut.await;
                                    Python::attach(|py| {
                                        py_result
                                            .map_err(|e: PyErr| e.to_string())
                                            .and_then(|py_val| {
                                                build_reply_body(
                                                    py,
                                                    &method.return_type,
                                                    py_val.bind(py),
                                                    &struct_map2,
                                                )
                                                .map_err(|e: PyErr| e.to_string())
                                                .map(|body| {
                                                    build_reply_frame(
                                                        &method_name,
                                                        seq_id,
                                                        &body,
                                                    )
                                                })
                                            })
                                    })
                                }
                            }
                        }
                    },
                };

                match result {
                    Ok(frame) => frame,
                    Err(err_msg) => {
                        build_exception_reply(&msg_begin.name, msg_begin.seq_id, 6, &err_msg)
                    }
                }
            }
        };

        if msg_begin.message_type != MESSAGE_TYPE_ONEWAY {
            send_response_async(&mut buf_writer, &response_payload, transport).await?;
        }
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Async framing helpers
// ──────────────────────────────────────────────────────────────────────────────

async fn send_response_async<W>(
    writer: &mut W,
    payload: &[u8],
    transport: TransportType,
) -> std::io::Result<()>
where
    W: AsyncWriteExt + Unpin,
{
    match transport {
        TransportType::Framed => {
            writer
                .write_i32(payload.len() as i32)
                .await?;
            writer.write_all(payload).await?;
        }
        TransportType::Buffered => {
            writer.write_all(payload).await?;
        }
    }
    writer.flush().await
}

// ──────────────────────────────────────────────────────────────────────────────
// Async buffered-transport body reader
// ──────────────────────────────────────────────────────────────────────────────

async fn read_buffered_struct_body_async<R>(
    reader: &mut R,
    out: &mut Vec<u8>,
) -> std::io::Result<()>
where
    R: AsyncReadExt + Unpin,
{
    loop {
        let field_type_byte = reader.read_u8().await?;
        out.push(field_type_byte);

        if field_type_byte == 0x00 {
            return Ok(());
        }

        let id_hi = reader.read_u8().await?;
        let id_lo = reader.read_u8().await?;
        out.push(id_hi);
        out.push(id_lo);

        read_buffered_value_async(reader, field_type_byte, out).await?;
    }
}

async fn read_buffered_value_async<R>(
    reader: &mut R,
    field_type_byte: u8,
    out: &mut Vec<u8>,
) -> std::io::Result<()>
where
    R: AsyncReadExt + Unpin,
{
    match field_type_byte {
        // BOOL, BYTE
        0x02 | 0x03 => {
            let b = reader.read_u8().await?;
            out.push(b);
        }
        // I16
        0x06 => {
            let mut buf = [0u8; 2];
            reader.read_exact(&mut buf).await?;
            out.extend_from_slice(&buf);
        }
        // I32
        0x08 => {
            let mut buf = [0u8; 4];
            reader.read_exact(&mut buf).await?;
            out.extend_from_slice(&buf);
        }
        // I64, DOUBLE
        0x0a | 0x04 => {
            let mut buf = [0u8; 8];
            reader.read_exact(&mut buf).await?;
            out.extend_from_slice(&buf);
        }
        // STRING / BINARY  (4-byte length prefix + data)
        0x0b => {
            let mut len_bytes = [0u8; 4];
            reader.read_exact(&mut len_bytes).await?;
            out.extend_from_slice(&len_bytes);
            let len = u32::from_be_bytes(len_bytes) as usize;
            let start = out.len();
            out.resize(start + len, 0);
            reader.read_exact(&mut out[start..]).await?;
        }
        // STRUCT (recursive)
        0x0c => {
            Box::pin(read_buffered_struct_body_async(reader, out)).await?;
        }
        // MAP
        0x0d => {
            let mut header = [0u8; 6];
            reader.read_exact(&mut header).await?;
            out.extend_from_slice(&header);
            let key_type = header[0];
            let val_type = header[1];
            let size = i32::from_be_bytes([header[2], header[3], header[4], header[5]]);
            for _ in 0..size {
                Box::pin(read_buffered_value_async(reader, key_type, out)).await?;
                Box::pin(read_buffered_value_async(reader, val_type, out)).await?;
            }
        }
        // LIST, SET
        0x0f | 0x0e => {
            let mut header = [0u8; 5];
            reader.read_exact(&mut header).await?;
            out.extend_from_slice(&header);
            let elem_type = header[0];
            let size = i32::from_be_bytes([header[1], header[2], header[3], header[4]]);
            for _ in 0..size {
                Box::pin(read_buffered_value_async(reader, elem_type, out)).await?;
            }
        }
        _ => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "Unknown field type byte 0x{:02x} in buffered stream",
                    field_type_byte
                ),
            ));
        }
    }
    Ok(())
}

// ──────────────────────────────────────────────────────────────────────────────
// Reply builders
// ──────────────────────────────────────────────────────────────────────────────

use crate::parser::ast::ThriftType;

fn build_reply_body(
    _py: Python<'_>,
    return_type: &ThriftType,
    value: &Bound<'_, PyAny>,
    struct_map: &HashMap<String, ThriftStruct>,
) -> PyResult<Vec<u8>> {
    let mut buf = Vec::with_capacity(64);
    let mut writer = BinaryProtocolWriter::new(&mut buf);

    match return_type {
        ThriftType::Struct(name) if name == "void" => {}
        _ => {
            let field_begin = FieldBegin {
                name: None,
                field_type: thrift_type_to_ttype(return_type),
                id: 0,
            };
            writer
                .write_field_begin(&field_begin)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
            write_value_with_structs(&mut writer, return_type, value, struct_map)?;
        }
    }

    writer
        .write_field_stop()
        .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
    Ok(buf)
}

fn build_reply_frame(method_name: &str, seq_id: i32, body: &[u8]) -> Vec<u8> {
    use crate::protocol::MESSAGE_TYPE_REPLY;
    let mut buf = Vec::with_capacity(32 + body.len());
    {
        let mut writer = BinaryProtocolWriter::new(&mut buf);
        writer
            .write_message_begin(&MessageBegin {
                name: method_name.to_string(),
                message_type: MESSAGE_TYPE_REPLY,
                seq_id,
            })
            .unwrap();
    }
    buf.extend_from_slice(body);
    buf
}

fn build_exception_reply(method_name: &str, seq_id: i32, ex_type: i32, message: &str) -> Vec<u8> {
    let mut buf = Vec::with_capacity(64);
    {
        let mut writer = BinaryProtocolWriter::new(&mut buf);
        writer
            .write_message_begin(&MessageBegin {
                name: method_name.to_string(),
                message_type: MESSAGE_TYPE_EXCEPTION,
                seq_id,
            })
            .unwrap();
        let msg_field = FieldBegin {
            name: None,
            field_type: TType::String,
            id: 1,
        };
        writer.write_field_begin(&msg_field).unwrap();
        writer.write_string(message).unwrap();
        let type_field = FieldBegin {
            name: None,
            field_type: TType::I32,
            id: 2,
        };
        writer.write_field_begin(&type_field).unwrap();
        writer.write_i32(ex_type).unwrap();
        writer.write_field_stop().unwrap();
    }
    buf
}

