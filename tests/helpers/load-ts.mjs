import fs from 'node:fs';
import path from 'node:path';
import vm from 'node:vm';
import ts from 'typescript';

// Tiny test loader for pure TS modules; external packages/DOM/Tauri are unavailable.
export function loadTs(file, cache = new Map()) {
  file = path.resolve(file);
  if (cache.has(file)) return cache.get(file).exports;
  const module = { exports: {} };
  cache.set(file, module);
  const compiled = ts.transpileModule(fs.readFileSync(file, 'utf8'), {
    compilerOptions: { target: ts.ScriptTarget.ES2020, module: ts.ModuleKind.CommonJS },
  }).outputText;
  vm.runInNewContext(compiled, {
    module, exports: module.exports,
    require(specifier) {
      if (!specifier.startsWith('.')) throw new Error(`Pure test module imported external dependency: ${specifier}`);
      const base = path.resolve(path.dirname(file), specifier);
      return loadTs(base.endsWith('.ts') ? base : `${base}.ts`, cache);
    },
  }, { filename: file });
  return module.exports;
}
