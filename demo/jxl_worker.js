async function workerMain() {
	importScripts("/jxl_wasm.js");

	const wasm = wasm_bindgen("/jxl_wasm_bg.wasm");

	self.addEventListener("fetch", async (event) => {
		if (event.request.method !== "GET") {
			console.log("not get");
			return;
		}

		event.respondWith((async () => {
			const res = await fetch(event.request);
			if (!res.ok)
				return res;

			let contentType = res.headers.get('Content-Type')
			if (contentType === 'application/octet-stream') {
				let disposition = res.headers.get('Content-Disposition')
				if (disposition && !disposition.toLowerCase().endsWith('.jxl"')) {
				  // Is not jxl image, nothing to decode here
				  return res
				}
			  } else if (contentType.toLowerCase() !== 'image/jxl') {
				// Is not jxl image, nothing to decode here
				return res
			  }

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
					"Content-Type": "image/apng",
				}
			})
		})());
	});
}

async function pageMain() {
	console.log("pagemain");
	if ("serviceWorker" in navigator) {
		navigator.serviceWorker.register("jxl_worker.js");
		const reg = await navigator.serviceWorker.ready;
		console.log("register worker");

		if (!navigator.serviceWorker.controller) {
			console.log("not navigator service worker");
			// ServiceWorker fetch event handler not ready yet, reload.
			setTimeout(() => {
				window.location.reload();
			}, 250);
		}
		console.log("end service worker in navigator");
	} else {
		console.log("service worker not in navigator");
	}
}

if (typeof window === "undefined") {
	workerMain();
	console.log("undefined");
} else {
	pageMain();
	console.log("not undefined");
}

