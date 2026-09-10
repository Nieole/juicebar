// 从设计稿导出图标基准，写进本目录。基准只能这样来：由设计稿自己的渲染器画、从它的画布上读回来——
// 由 Rust 生成再拿回来当基准，比对就成了自己比自己。
//
// 用法（仓库根目录）：node tests/icon_baselines/export.mjs [浏览器路径]
// 不给路径就依次找本机的 Chrome 与 Edge。它用无头浏览器打开 icon-review.html#baselines，
// 把页底 <pre id="baselines"> 里的 JSON 拆成文件；有任何一个像素认不出是调色板里的哪一格，就一个文件都不写。
import { execFileSync } from 'node:child_process';
import { existsSync, mkdtempSync, readdirSync, rmSync, unlinkSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const page = resolve(here, '../../.scratch/tray-design/icon-review.html');
const browser = [
  process.argv[2],
  'C:/Program Files/Google/Chrome/Application/chrome.exe',
  'C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe',
].find((p) => p && existsSync(p));
if (!browser) throw new Error('找不到浏览器；把 Chrome 或 Edge 的路径作为第一个参数传进来');

const profile = mkdtempSync(join(tmpdir(), 'juicebar-baselines-'));
let dom;
try {
  dom = execFileSync(browser, [
    '--headless=new', '--disable-gpu', '--no-first-run', '--no-default-browser-check',
    `--user-data-dir=${profile}`, '--dump-dom', `${pathToFileURL(page).href}#baselines`,
  ], { encoding: 'utf8', maxBuffer: 256 * 1024 * 1024 });
} finally {
  rmSync(profile, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
}

const m = dom.match(/<pre id="baselines">([\s\S]*?)<\/pre>/);
if (!m) throw new Error('页面里没有 <pre id="baselines">：导出入口没跑起来');
const text = m[1].replace(/&lt;/g, '<').replace(/&gt;/g, '>').replace(/&amp;/g, '&');
const { files, unmatched } = JSON.parse(text);
if (unmatched) throw new Error(`${unmatched} 个像素认不出是调色板里的哪一格（网格里的“?”），一个文件都没写`);

for (const name of readdirSync(here)) if (name.endsWith('.txt')) unlinkSync(join(here, name));
let grids = 0;
for (const [name, body] of Object.entries(files)) {
  writeFileSync(join(here, name), body);
  grids += (body.match(/^= /gm) || []).length;
}
console.log(`写了 ${Object.keys(files).length} 个文件、${grids} 张网格，来自 ${browser}`);
