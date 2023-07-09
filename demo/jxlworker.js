async function workerMain() {
	importScripts("../pkg/jxltranscode.js");

	const wasm = wasm_bindgen("../pkg/jxltranscode_bg.wasm");

	self.addEventListener("fetch", async (event) => {
		if (event.request.method !== "GET")
			return;

		event.respondWith((async () => {
			const res = await fetch(event.request);
			if (!res.ok)
				return res;

			// Check if this is a JXL image.
			const contentType = res.headers.get("Content-Type");
			if (contentType != "image/jxl")
				return res;

			// Wait for WASM to finish loading.
			await wasm;

			// Load image into RAM.
			const jxl = new Uint8Array(await (await res.blob()).arrayBuffer());

			// Create a stream for the output.
			let streamController;
			const stream = new ReadableStream({
				start: (controller) => {
					streamController = controller;
				}
			});

			setTimeout(() => {
				console.log("Decoding...");
				// Transcode.
				const t0 = Date.now();
				wasm_bindgen.transcode(jxl, (buf) => {
					streamController.enqueue(buf);
				});
				const t1 = Date.now();

				console.log(`Decoding took ${t1 - t0} ms`);
				streamController.close();
			}, 0);

			console.log("Responding...");
			return new Response(stream, {
				status: res.status,
				statusText: res.statusText,
				headers: {
					"Content-Type": "image/png",
				}
			})
		})());
	});
}

async function pageMain() {
	if ("serviceWorker" in navigator) {
		navigator.serviceWorker.register("jxlworker.js");
		const reg = await navigator.serviceWorker.ready;

		if (!navigator.serviceWorker.controller) {
			// ServiceWorker fetch event handler not ready yet, reload.
			setTimeout(() => {
				window.location.reload();
			}, 250);
		}
	}
}

if (typeof window === "undefined") {
	workerMain();
} else {
	pageMain();
}

