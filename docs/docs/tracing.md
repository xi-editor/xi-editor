# Tracing

To debug or find performance bottlenecks in xi, the crate `xi_trace` can be used to write a trace log.

## Analyzing the Trace Log

To write the trace log into a file in [xi-mac](https://github.com/xi-editor/xi-mac): Debug → Write Trace (F5)

The create file can be opened and analyzed in Chrome in `about:tracing`.

## Trace Methods

All methods are available in [`xi_trace`](https://github.com/xi-editor/xi-editor/blob/master/rust/trace/src/lib.rs).

### trace

Sample without any payload.

`xi_trace::trace("something happened", &["rpc", "response"]);`

### trace_payload

Sample with payload.

`xi_trace::trace_payload("my event", &["rpc", "response"], json!({"key": "value"}));`

### trace_block and trace_block_payload

Duration sample.

```
let trace_guard = xi_trace::trace_block("something_expensive", &["rpc", "request"]);
something_expensive();
std::mem::drop(trace_guard); // finalize explicitly
```

### trace_closure and trace_closure_payload

Duration sample that measures how long the closure took to execute.

```
xi_trace::trace_closure("something_else_expensive", &["rpc", "response"], || {
    something_else_expensive(result);
});
```
