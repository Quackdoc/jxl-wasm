jxl-oxide wasm demo
===================

Small demo project showing how to transparently transcode JpegXL images via a WebAssembly-based ServiceWorker.
This acts as a sort of polyfill for browsers without native JpegXL decoding support.

Running
-------

Run

```bash
wasm-pack build --target no-modules
python -m http.server
```

Then open http://localhost:8000/demo/index.html
