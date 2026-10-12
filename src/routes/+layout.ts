/**
 * The frontend is a client-rendered SPA inside a Tauri WebView: there is no server at
 * runtime, so SSR is off and nothing is prerendered.
 *
 * Nothing is prerendered on purpose. With `adapter-static`'s `index.html` fallback, the
 * build emits one application shell that Tauri loads from disk and that SvelteKit's client
 * router drives for every route — including deep links like `/analyze/<id>`, whose ids only
 * exist after a library has been indexed and so could never be prerendered anyway.
 * Prerendering the static routes as well would only race the fallback for `index.html`.
 */
export const prerender = false;
export const ssr = false;
