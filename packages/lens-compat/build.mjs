import { build } from "esbuild";
await build({
  absWorkingDir: import.meta.dirname,
  entryPoints: ["src/runtime.jsx"],
  bundle: true,
  outfile: "dist/runtime.js",
  format: "iife",
  globalName: "FreelensRuntime",
  platform: "browser",
  target: "es2021",
  define: { "process.env.NODE_ENV": '"production"' },
  minify: true,
});
