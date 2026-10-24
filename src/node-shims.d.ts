/**
 * Minimal typings for the one Node API the frontend's tests use.
 *
 * This project deliberately has no `@types/node`: it is a browser-only frontend, and pulling the
 * whole Node type surface in would let a component import `fs` and still type-check. But
 * `theme.test.ts` has to read `src/app.css` as text, and a `?raw` import cannot do it — the
 * Tailwind Vite plugin owns `.css` requests and returns an empty module for them (verified: the
 * import type-checks, resolves, and is `''` at runtime). So the test reads the file, and this is
 * the narrowest possible declaration to let it.
 */
declare module 'node:fs' {
	export function readFileSync(path: URL | string, encoding: 'utf8'): string;
}
