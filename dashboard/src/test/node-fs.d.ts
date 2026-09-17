// The app's tsconfig carries no Node types, but tests run under Node. This is
// the little of Node a test needs: reading a source file that a CSS import would
// hand back empty (vitest resolves `*.css?raw` to an empty module), and walking
// the source tree for the tests that assert something is written nowhere.
declare module 'node:fs' {
  export function readFileSync(path: string, encoding: 'utf8'): string;
  export function readdirSync(path: string): string[];
  export function statSync(path: string): { isDirectory(): boolean };
}

declare module 'node:path' {
  export function join(...parts: string[]): string;
}
