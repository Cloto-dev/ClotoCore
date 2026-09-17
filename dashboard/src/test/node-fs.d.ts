// The app's tsconfig carries no Node types, but tests run under Node. This is
// the one Node API a test needs: reading a source file that a CSS import would
// hand back empty (vitest resolves `*.css?raw` to an empty module).
declare module 'node:fs' {
  export function readFileSync(path: string, encoding: 'utf8'): string;
}
