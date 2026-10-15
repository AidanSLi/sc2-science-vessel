/**
 * Public surface of the frontend library.
 *
 * Routes import from `#lib/<module>` directly (SvelteKit 3's alias is Node subpath imports
 * declared in `package.json`, replacing the old implicit `$lib`), and this barrel exists
 * for code that wants the whole surface at once. Keep the ad-hoc exports below
 * alphabetical so it stays scannable.
 */

export * from './analysis';
export * from './analysis-view';
export * from './build-library';
export * from './builds';
export * from './charts';
export * from './clock';
export * from './client-api';
export * from './sample-build';
export type * from './types';
