/**
 * `@srelens/ui-next/styles` resolves to a CSS file behind a package export.
 * `vite/client` types bare `*.css` specifiers, but not a subpath export that
 * happens to point at one, so TypeScript needs telling it is importable for
 * its side effect alone.
 */
declare module "@srelens/ui-next/styles";
