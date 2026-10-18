import * as maplibregl from "maplibre-gl";
// MapLibre 6 runs tiles in a separate ES-module worker; let Vite bundle it and hand over its URL.
import workerUrl from "maplibre-gl/dist/maplibre-gl-worker.mjs?worker&url";

maplibregl.setWorkerUrl(workerUrl);

export { maplibregl };
