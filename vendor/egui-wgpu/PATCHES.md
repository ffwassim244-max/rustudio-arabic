# egui-wgpu 0.36.2, as vendored by rustudio

One change, in `src/renderer.rs`, each place marked `rustudio patch`:

- `Renderer::update_buffers` no longer panics when `Queue::write_buffer_with`
  gives no staging buffer for the frame's index or vertex data (wgpu refuses
  one when the GPU has no memory for it or the device is lost; wgpu reports
  the error to the device's uncaptured-error handler). It logs the refusal,
  marks the renderer `starved` for the frame and returns no command buffers;
  `Renderer::render` then draws nothing for that frame. The next
  `update_buffers` tries again.

Why: on an Android phone whose GPU driver had just been reset, the panic
(`renderer.rs:984 Failed to create staging buffer for index data`) ended the
process with the open place; the frame could have gone on, and the app can
now notice the lost device and save its work (src/android/mod.rs
`install_gpu_error_handler`, src/app.rs).
