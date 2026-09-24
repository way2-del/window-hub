import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import ts from 'typescript';

export const root = fileURLToPath(new URL('../../', import.meta.url));
export function sourceFiles(dir) {
  return fs.readdirSync(dir, { withFileTypes: true }).flatMap(entry => {
    const file = path.join(dir, entry.name);
    return entry.isDirectory() ? sourceFiles(file) : /\.(ts|tsx)$/.test(file) && !file.endsWith('.d.ts') ? [file] : [];
  });
}

// Runtime imports/re-exports/dynamic imports only. Type-only edges cannot execute a cycle.
export function imports(source, file) {
  const ast = ts.createSourceFile(file, source, ts.ScriptTarget.Latest, true);
  const result = [];
  function visit(node) {
    if (ts.isImportDeclaration(node) && !node.importClause?.isTypeOnly) {
      const bindings = node.importClause?.namedBindings;
      const typesOnly = bindings && ts.isNamedImports(bindings) && bindings.elements.length > 0 &&
        bindings.elements.every(e => e.isTypeOnly) && !node.importClause.name;
      if (!typesOnly) result.push(node.moduleSpecifier.text);
    } else if (ts.isExportDeclaration(node) && node.moduleSpecifier && !node.isTypeOnly) {
      const typesOnly = node.exportClause && ts.isNamedExports(node.exportClause) &&
        node.exportClause.elements.length > 0 && node.exportClause.elements.every(e => e.isTypeOnly);
      if (!typesOnly) result.push(node.moduleSpecifier.text);
    } else if (ts.isCallExpression(node) && node.expression.kind === ts.SyntaxKind.ImportKeyword &&
               node.arguments[0] && ts.isStringLiteral(node.arguments[0])) {
      result.push(node.arguments[0].text);
    }
    ts.forEachChild(node, visit);
  }
  visit(ast);
  return result;
}

export function graphFor(projectRoot = root) {
  const files = sourceFiles(path.join(projectRoot, 'src'));
  const authoring = path.join(projectRoot, 'plugins');
  if (fs.existsSync(authoring)) files.push(...sourceFiles(authoring));
  const names = new Set(files.map(f => path.relative(projectRoot, f).replaceAll('\\', '/')));
  return new Map([...names].sort().map(file => {
    const deps = imports(fs.readFileSync(path.join(projectRoot, file), 'utf8'), file)
      .filter(spec => spec.startsWith('.')).map(spec => {
        const base = path.posix.normalize(path.posix.join(path.posix.dirname(file), spec));
        return [base, `${base}.ts`, `${base}.tsx`, `${base}/index.ts`, `${base}/index.tsx`].find(p => names.has(p));
      }).filter(Boolean);
    return [file, [...new Set(deps)]];
  }));
}

export function cyclicEdges(graph) {
  const reaches = (start, target, seen = new Set()) => {
    if (start === target) return true;
    if (seen.has(start)) return false;
    seen.add(start);
    return (graph.get(start) ?? []).some(next => reaches(next, target, seen));
  };
  return [...graph].flatMap(([from, deps]) => deps.filter(to => reaches(to, from))
    .map(to => `${from} -> ${to}`)).sort();
}

export function boundaryViolations(graph) {
  const entries = /^(src\/[^/]*App\.tsx|src\/App\.tsx|src\/components\/PluginPopupHost\.tsx)$/;
  return [...graph].flatMap(([from, deps]) => deps.filter(to =>
    (entries.test(to) && from !== 'src/main.tsx') ||
    (from.startsWith('src/features/chrome/') && !to.startsWith('src/features/chrome/')) ||
    (from.startsWith('src/features/island/') && !to.startsWith('src/features/island/')) ||
    (from.startsWith('plugins/') && to.startsWith('src/')))
    .map(to => `${from} -> ${to}`)).sort();
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const graph = graphFor();
  const cycles = cyclicEdges(graph);
  const entries = boundaryViolations(graph);
  const failures = [
    ...cycles.map(edge => `Runtime cycle: ${edge}`),
    ...entries.map(edge => `Module boundary violation: ${edge}`),
  ];
  for (const pure of ['src/app/windowRouting.ts', 'src/features/chrome/tokens.ts',
    'src/features/island/motion.ts', 'src/features/island/pullContent.ts']) {
    if (imports(fs.readFileSync(path.join(root, pure), 'utf8'), pure).length) {
      failures.push(`${pure} must stay pure (no runtime imports).`);
    }
  }
  console.log(`Architecture: ${graph.size} modules, ${cycles.length} cyclic edges, ${entries.length} invalid entry imports.`);
  for (const failure of failures) console.error(failure);
  process.exitCode = failures.length ? 1 : 0;
}
