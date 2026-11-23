// Reuse the locked editor bundle, exposing the garden's hover and indentation extensions.
import {readFileSync} from 'node:fs';
import {resolve, join} from 'node:path';
import {createRequire} from 'node:module';
import {fileURLToPath} from 'node:url';

const vendor = resolve(process.argv[2]);
const require = createRequire(join(vendor, 'package.json'));
const licenseBanner = readFileSync(join(vendor, 'banner.txt'), 'utf8').trimEnd();
const banner = licenseBanner.replace(/^\/\*!\n(?: \* .+\n)+/, '/*!\n * CodeMirror 6 for the Rustia web editor.\n');
const entry = readFileSync(join(vendor, 'codemirror.entry.js'), 'utf8');
await require('esbuild').build({
  stdin: {contents: entry + "\nexport {hoverTooltip} from '@codemirror/view';\nexport {indentUnit} from '@codemirror/language';", resolveDir: vendor},
  bundle: true, format: 'esm', minify: true, legalComments: 'inline',
  banner: {js: banner},
  outfile: fileURLToPath(new URL('../../codemirror.js', import.meta.url))
});
