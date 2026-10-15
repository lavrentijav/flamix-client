#!/usr/bin/env node
// Extract plain SVG files from the Alfa-Bank (@alfalab) React icon packages.
//
// The packages ship CommonJS modules that build a React element tree via
// `createElement`. We inject a fake `react` module whose createElement returns
// a plain {type, props, children} tree, then serialise that tree to a clean SVG
// string (camelCase React attrs -> kebab-case SVG attrs, React-only props dropped).
//
// Usage:
//   node scripts/icons/extract.mjs <glyph-pkg-dir> <flag-pkg-dir> <out-dir>
//
// <glyph-pkg-dir> / <flag-pkg-dir> are the unpacked package roots (containing
// *MIcon.js / *MColorIcon.js). <out-dir> is the repo assets dir, i.e. app/flamix/assets.

import { createRequire } from 'node:module';
import Module from 'node:module';
import fs from 'node:fs';
import path from 'node:path';
import { spawnSync } from 'node:child_process';

const require = createRequire(import.meta.url);

// ---------------------------------------------------------------------------
// Fake react
// ---------------------------------------------------------------------------

const createElement = (type, props, ...children) => {
  if (typeof type === 'function') {
    return type({ ...(props || {}), children: children.length === 1 ? children[0] : children });
  }
  return { type, props: props || {}, children: children.flat(Infinity) };
};

const fakeReact = { createElement };
fakeReact.default = fakeReact;
fakeReact.__esModule = true;

const origLoad = Module._load;
Module._load = function (request, parent, isMain) {
  if (request === 'react' || request === 'react-dom') return fakeReact;
  return origLoad.apply(this, arguments);
};

// ---------------------------------------------------------------------------
// Attribute handling
// ---------------------------------------------------------------------------

// React prop name -> canonical SVG attribute name (needs a hyphen).
const ATTR_MAP = {
  fillRule: 'fill-rule',
  clipRule: 'clip-rule',
  clipPath: 'clip-path',
  strokeWidth: 'stroke-width',
  strokeLinecap: 'stroke-linecap',
  strokeLinejoin: 'stroke-linejoin',
  strokeDasharray: 'stroke-dasharray',
  strokeDashoffset: 'stroke-dashoffset',
  strokeMiterlimit: 'stroke-miterlimit',
  strokeOpacity: 'stroke-opacity',
  fillOpacity: 'fill-opacity',
  stopColor: 'stop-color',
  stopOpacity: 'stop-opacity',
  colorInterpolation: 'color-interpolation',
  colorInterpolationFilters: 'color-interpolation-filters',
  textAnchor: 'text-anchor',
  dominantBaseline: 'dominant-baseline',
  fontFamily: 'font-family',
  fontSize: 'font-size',
  fontWeight: 'font-weight',
  xlinkHref: 'xlink:href',
  floodColor: 'flood-color',
  floodOpacity: 'flood-opacity',
  lightingColor: 'lighting-color',
  markerStart: 'marker-start',
  markerMid: 'marker-mid',
  markerEnd: 'marker-end',
  maskType: 'mask-type',
};

// Native SVG attributes that are already camelCase and must stay as-is.
const KEEP_CAMEL = new Set([
  'viewBox',
  'preserveAspectRatio',
  'gradientUnits',
  'gradientTransform',
  'patternUnits',
  'patternContentUnits',
  'patternTransform',
  'maskUnits',
  'maskContentUnits',
  'clipPathUnits',
  'markerWidth',
  'markerHeight',
  'markerUnits',
  'refX',
  'refY',
  'pathLength',
  'textLength',
  'lengthAdjust',
  'spreadMethod',
  'baseFrequency',
  'numOctaves',
  'stdDeviation',
  'surfaceScale',
  'specularConstant',
  'specularExponent',
  'diffuseConstant',
  'kernelMatrix',
  'kernelUnitLength',
  'tableValues',
  'targetX',
  'targetY',
]);

// Props that exist only in the React tree, not in an SVG file.
const DROP_PROPS = new Set(['key', 'ref', 'dangerouslySetInnerHTML', 'children', 'role', 'focusable']);

function attrName(name) {
  if (ATTR_MAP[name]) return ATTR_MAP[name];
  if (KEEP_CAMEL.has(name)) return name;
  if (name.includes(':')) return name;
  if (/[A-Z]/.test(name)) return name.replace(/[A-Z]/g, (c) => '-' + c.toLowerCase());
  return name;
}

function styleToString(obj) {
  return Object.entries(obj)
    .map(([k, v]) => `${k.replace(/[A-Z]/g, (c) => '-' + c.toLowerCase())}:${v}`)
    .join(';');
}

function escapeAttr(v) {
  return String(v)
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;');
}

function escapeText(v) {
  return v.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
}

// ---------------------------------------------------------------------------
// Serialisation
// ---------------------------------------------------------------------------

function serialize(node, { root = false, monochrome = false } = {}) {
  if (node === null || node === undefined || node === false || node === true) return '';
  if (typeof node === 'string' || typeof node === 'number') return escapeText(String(node));
  if (Array.isArray(node)) return node.map((c) => serialize(c, { monochrome })).join('');

  const { type, props, children } = node;
  const attrs = [];
  if (root) attrs.push(['xmlns', 'http://www.w3.org/2000/svg']);

  for (const [key, raw] of Object.entries(props || {})) {
    if (DROP_PROPS.has(key)) continue;
    if (raw === undefined || raw === null || raw === false) continue;
    let value = raw;
    if (key === 'style' && typeof raw === 'object') value = styleToString(raw);
    if (monochrome && value === 'currentColor') value = '#000000';
    attrs.push([attrName(key), value]);
  }

  const attrStr = attrs.map(([k, v]) => ` ${k}="${escapeAttr(v)}"`).join('');
  const inner = (children || []).map((c) => serialize(c, { monochrome })).join('');
  if (!inner) return `<${type}${attrStr}/>`;
  return `<${type}${attrStr}>${inner}</${type}>`;
}

function iconToSvg(modulePath, { monochrome }) {
  delete require.cache[require.resolve(modulePath)];
  const mod = require(modulePath);
  const component = mod.default || mod;
  const tree = component({});
  const body = serialize(tree, { root: true, monochrome });
  return `<?xml version="1.0" encoding="UTF-8"?>\n${body}\n`;
}

// ---------------------------------------------------------------------------
// Semantic glyph mapping
// ---------------------------------------------------------------------------

// semantic file name -> Alfa glyph component base name (M = 24px size).
const GLYPHS = {
  'sidebar-expand': 'ArrowRight',
  'sidebar-collapse': 'ArrowLeft',
  add: 'PlusCircle',
  routing: 'WorldSegmented',
  settings: 'Gear',
  stats: 'StatsChart',
  info: 'InformationCircle',
  search: 'Magnifier',
  edit: 'Pencil',
  more: 'DotsHorizontal',
  'chevron-down': 'ChevronDown',
  'chevron-right': 'ChevronRight',
  'chevron-up': 'ChevronUp',
  pin: 'Pin',
  refresh: 'ArrowsCw',
  power: 'PowerButton',
  ping: 'Speed',
  copy: 'Copy',
  paste: 'CopyLine',
  trash: 'TrashCan',
  qr: 'QrCode',
  link: 'Link',
  check: 'Checkmark',
  close: 'Cross',
  download: 'Download',
  upload: 'ArrowUp',
  clock: 'Clock',
  shield: 'Shield',
  key: 'Key',
  lock: 'LockClosed',
  sun: 'Sun',
  moon: 'Moon',
  globe: 'World',
  filter: 'Filter',
  sort: 'ArrowsUpDown',
  list: 'List',
  warning: 'ExclamationCircle',
  support: 'QuestionCircle',
  web: 'LinkCircle',
};

// ---------------------------------------------------------------------------
// Flag name -> ISO 3166-1 alpha-2 (upper case)
// ---------------------------------------------------------------------------

const FLAGS = {
  Afghanistan: 'AF', AlandIslands: 'AX', Albania: 'AL', Algeria: 'DZ', AmericanSamoa: 'AS',
  Andorra: 'AD', Angola: 'AO', Anguilla: 'AI', Antarctica: 'AQ', AntiguaAndBarbuda: 'AG',
  Argentina: 'AR', Armenia: 'AM', Aruba: 'AW', Australia: 'AU', Austria: 'AT', Azerbaijan: 'AZ',
  Bahamas: 'BS', Bahrain: 'BH', Bangladesh: 'BD', Barbados: 'BB', Belarus: 'BY', Belgium: 'BE',
  Belize: 'BZ', Benin: 'BJ', Bermuda: 'BM', Bhutan: 'BT', Bolivia: 'BO', Bonaire: 'BQ',
  BosniaAndHerzegovina: 'BA', Botswana: 'BW', Brazil: 'BR', BritishIndianOceanTerritory: 'IO',
  BritishVirginIslands: 'VG', Brunei: 'BN', Bulgaria: 'BG', BurkinaFaso: 'BF', Burundi: 'BI',
  Cambodia: 'KH', Cameroon: 'CM', Canada: 'CA', CapeVerde: 'CV', CaymanIslands: 'KY',
  CentralAfricanRepublic: 'CF', Chad: 'TD', Chile: 'CL', China: 'CN', ChristmasIsland: 'CX',
  CocosIsland: 'CC', Colombia: 'CO', Comoros: 'KM', CookIslands: 'CK', CostaRica: 'CR',
  Croatia: 'HR', Cuba: 'CU', Curacao: 'CW', Cyprus: 'CY', CzechRepublic: 'CZ',
  DemocraticRepublicOfCongo: 'CD', Denmark: 'DK', Djibouti: 'DJ', Dominica: 'DM',
  DominicanRepublic: 'DO', EastTimor: 'TL', Ecuador: 'EC', Egypt: 'EG', ElSalvador: 'SV',
  EquatorialGuinea: 'GQ', Eritrea: 'ER', Estonia: 'EE', Ethiopia: 'ET', EuropeanUnion: 'EU',
  FalklandIslands: 'FK', FaroeIslands: 'FO', Fiji: 'FJ', Finland: 'FI', France: 'FR',
  FrenchPolynesia: 'PF', Gabon: 'GA', Gambia: 'GM', Georgia: 'GE', Germany: 'DE', Ghana: 'GH',
  Gibraltar: 'GI', Greece: 'GR', Greenland: 'GL', Grenada: 'GD', Guam: 'GU', Guatemala: 'GT',
  Guernsey: 'GG', GuineaBissau: 'GW', Guinea: 'GN', Guyana: 'GY', Haiti: 'HT', Honduras: 'HN',
  HongKong: 'HK', Hungary: 'HU', Iceland: 'IS', India: 'IN', Indonesia: 'ID', Iran: 'IR',
  Iraq: 'IQ', Ireland: 'IE', IsleOfMan: 'IM', Israel: 'IL', Italy: 'IT', IvoryCoast: 'CI',
  Jamaica: 'JM', Japan: 'JP', Jersey: 'JE', Jordan: 'JO', Kazakhstan: 'KZ', Kenya: 'KE',
  Kiribati: 'KI', Kosovo: 'XK', Kuwait: 'KW', Kwait: 'KW', Kyrgyzstan: 'KG', Laos: 'LA',
  Latvia: 'LV', Lebanon: 'LB', Lesotho: 'LS', Liberia: 'LR', Libya: 'LY', Liechtenstein: 'LI',
  Lithuania: 'LT', Luxembourg: 'LU', Macao: 'MO', Madagascar: 'MG', Malasya: 'MY', Malawi: 'MW',
  Malaysia: 'MY', Maldives: 'MV', Mali: 'ML', Malta: 'MT', MarshallIsland: 'MH', Martinique: 'MQ',
  Mauritania: 'MR', Mauritius: 'MU', Mexico: 'MX', Micronesia: 'FM', Moldova: 'MD', Monaco: 'MC',
  Mongolia: 'MN', Montenegro: 'ME', Montserrat: 'MS', Morocco: 'MA', Mozambique: 'MZ',
  Myanmar: 'MM', Namibia: 'NA', Nauru: 'NR', Nepal: 'NP', Netherlands: 'NL', NewZealand: 'NZ',
  Nicaragua: 'NI', Niger: 'NE', Nigeria: 'NG', Niue: 'NU', NorfolkIsland: 'NF', NorthKorea: 'KP',
  NorthernMarianasIslands: 'MP', Norway: 'NO', Oman: 'OM', Pakistan: 'PK', Palau: 'PW',
  Palestine: 'PS', Panama: 'PA', PapuaNewGuinea: 'PG', Paraguay: 'PY', Peru: 'PE',
  Philippines: 'PH', PitcairnIslands: 'PN', Poland: 'PL', Portugal: 'PT', PuertoRico: 'PR',
  Qatar: 'QA', RepublicOfMacedonia: 'MK', RepublicOfTheCongo: 'CG', Romania: 'RO', Russia: 'RU',
  Rwanda: 'RW', SabaIsland: 'BQ', SahrawiArabDemocraticRepublic: 'EH', SaintKittsAndNevis: 'KN',
  Samoa: 'WS', SanMarino: 'SM', SaoTomeAndPrince: 'ST', SaoTomeAndPrincipe: 'ST',
  SaudiArabia: 'SA', Senegal: 'SN', Serbia: 'RS', Seychelles: 'SC', SierraLeone: 'SL',
  Singapore: 'SG', SintEustatius: 'BQ', SintMaarten: 'SX', Slovakia: 'SK', Slovenia: 'SI',
  SolomonIslands: 'SB', Somalia: 'SO', SouthAfrica: 'ZA', SouthKorea: 'KR', SouthSudan: 'SS',
  Spain: 'ES', SriLanka: 'LK', StBarts: 'BL', StLucia: 'LC', StVincentAndTheGrenadines: 'VC',
  Sudan: 'SD', Suriname: 'SR', Swaziland: 'SZ', Sweden: 'SE', Switzerland: 'CH', Syria: 'SY',
  Taiwan: 'TW', Tajikistan: 'TJ', Tanzania: 'TZ', Thailand: 'TH', Togo: 'TG', Tokelau: 'TK',
  Tonga: 'TO', TrinidadAndTobago: 'TT', Tunisia: 'TN', Turkey: 'TR', Turkmenistan: 'TM',
  TurksAndCaicos: 'TC', Tuvalu1: 'TV', Tuvalu: 'TV', Uganda: 'UG', Ukraine: 'UA',
  UnitedArabEmirates: 'AE', UnitedKingdom: 'GB', UnitedStates: 'US', Uruguay: 'UY',
  Uzbekistn: 'UZ', Vanuatu: 'VU', VaticanCity: 'VA', Venezuela: 'VE', Vietnam: 'VN',
  VirginIslands: 'VI', Yemen: 'YE', Zambia: 'ZM', Zimbabwe: 'ZW',
};

// ---------------------------------------------------------------------------
// App icon
// ---------------------------------------------------------------------------

function appIconSvg(powerPath) {
  const s = 1024;
  return `<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" width="${s}" height="${s}" viewBox="0 0 ${s} ${s}">
  <defs>
    <linearGradient id="bg" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="#C8862E"/>
      <stop offset="1" stop-color="#7A4A12"/>
    </linearGradient>
  </defs>
  <rect width="${s}" height="${s}" rx="225" fill="url(#bg)"/>
  <g transform="translate(256 256) scale(21.3333333)" fill="#FFFFFF">
    ${powerPath}
  </g>
</svg>
`;
}

// Extract the inner markup of a generated 24x24 glyph, dropping the svg wrapper
// and the black fill so it can inherit the group's white fill.
function innerMarkup(svg) {
  const m = svg.replace(/^<\?xml[^>]*\?>\s*/, '').match(/<svg[^>]*>([\s\S]*)<\/svg>\s*$/);
  if (!m) throw new Error('cannot find svg body');
  return m[1].replace(/ fill="#000000"/g, '').trim();
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

function main() {
  const [glyphDir, flagDir, outDir] = process.argv.slice(2);
  if (!glyphDir || !flagDir || !outDir) {
    console.error('usage: node scripts/icons/extract.mjs <glyph-pkg-dir> <flag-pkg-dir> <out-dir>');
    process.exit(2);
  }

  const iconsDir = path.join(outDir, 'icons');
  const flagsDir = path.join(outDir, 'flags');
  const appIconDir = path.join(outDir, 'app-icon');
  fs.mkdirSync(iconsDir, { recursive: true });
  fs.mkdirSync(flagsDir, { recursive: true });
  fs.mkdirSync(appIconDir, { recursive: true });

  // ---- glyphs ----
  const glyphRows = [];
  for (const [name, comp] of Object.entries(GLYPHS)) {
    const modPath = path.join(glyphDir, `${comp}MIcon.js`);
    if (!fs.existsSync(modPath)) throw new Error(`missing glyph ${comp} for ${name}`);
    const svg = iconToSvg(modPath, { monochrome: true });
    fs.writeFileSync(path.join(iconsDir, `${name}.svg`), svg);
    glyphRows.push([name, comp]);
  }

  // ---- flags ----
  const flagFiles = fs
    .readdirSync(flagDir)
    .filter((f) => /MColorIcon\.js$/.test(f) && !/\.d\.js$/.test(f));
  const unmatched = [];
  let flagCount = 0;
  for (const file of flagFiles) {
    const base = file.replace(/MColorIcon\.js$/, '');
    const iso = FLAGS[base];
    if (!iso) {
      unmatched.push(base);
      continue;
    }
    const svg = iconToSvg(path.join(flagDir, file), { monochrome: false });
    fs.writeFileSync(path.join(flagsDir, `${iso}.svg`), svg);
    flagCount++;
  }
  unmatched.sort();

  // ---- neutral fallback flag ----
  const unknown = `<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none">
  <circle cx="12" cy="12" r="10" fill="#9E9E9E"/>
  <path d="M2 12h20M12 2c2.5 2.7 4 6.2 4 10s-1.5 7.3-4 10c-2.5-2.7-4-6.2-4-10s1.5-7.3 4-10z" stroke="#FFFFFF" stroke-width="1.5"/>
</svg>
`;
  fs.writeFileSync(path.join(flagsDir, 'UN.svg'), unknown);

  // ---- app icon ----
  const powerSvg = iconToSvg(path.join(glyphDir, 'PowerButtonMIcon.js'), { monochrome: true });
  const appSvg = appIconSvg(innerMarkup(powerSvg));
  fs.writeFileSync(path.join(outDir, 'app-icon.svg'), appSvg);

  // ---- PNGs (best effort) ----
  const sizes = [16, 32, 48, 64, 128, 256, 512, 1024];
  const has = (bin) => spawnSync('sh', ['-c', `command -v ${bin}`]).status === 0;
  const renderer = has('rsvg-convert') ? 'rsvg' : has('convert') ? 'imagemagick' : null;
  const pngNote = renderer
    ? `rendered with ${renderer === 'rsvg' ? 'rsvg-convert' : 'ImageMagick'}`
    : 'no rsvg-convert / ImageMagick found, PNGs skipped';
  if (renderer) {
    for (const size of sizes) {
      const out = path.join(appIconDir, `${size}.png`);
      if (renderer === 'rsvg') {
        const r = spawnSync('rsvg-convert', ['-w', String(size), '-h', String(size), '-o', out, path.join(outDir, 'app-icon.svg')]);
        if (r.status !== 0) throw new Error('rsvg-convert failed: ' + r.stderr);
      } else {
        const r = spawnSync('convert', ['-background', 'none', '-resize', `${size}x${size}`, path.join(outDir, 'app-icon.svg'), out]);
        if (r.status !== 0) throw new Error('convert failed: ' + r.stderr);
      }
    }
  }

  // ---- SOURCES.md ----
  const sources = [];
  sources.push('# Icon sources\n');
  sources.push('Generated by `scripts/icons/extract.mjs` from the Alfa-Bank open icon packs');
  sources.push('([@alfalab/icons-glyph](https://www.npmjs.com/package/@alfalab/icons-glyph)');
  sources.push('and [@alfalab/icons-flag](https://www.npmjs.com/package/@alfalab/icons-flag), MIT).\n');
  sources.push('## Glyphs (`assets/icons/`)\n');
  sources.push('| semantic name | source component |');
  sources.push('|---|---|');
  for (const [name, comp] of glyphRows) sources.push(`| ${name}.svg | ${comp}MIcon |`);
  sources.push('\nMonochrome 24x24 (`*MIcon`) variants; `currentColor` replaced with `#000000`.\n');
  if (unmatched.length) {
    sources.push('## Flags without an ISO 3166-1 alpha-2 code\n');
    sources.push('These package entries are subdivisions, historical or unofficial entities and have no alpha-2 code:');
    sources.push(unmatched.join(', ') + '.\n');
  } else {
    sources.push('## Flags\n\nAll flag entries matched an ISO 3166-1 alpha-2 code.\n');
  }
  sources.push('`assets/flags/UN.svg` is a neutral grey globe used for unknown regions.\n');
  sources.push(`App icon PNGs: ${pngNote}.\n`);
  fs.writeFileSync(path.join(outDir, 'SOURCES.md'), sources.join('\n'));

  console.log(`glyphs: ${glyphRows.length}`);
  console.log(`flags: ${flagCount} (ISO files), unmatched: ${unmatched.length}`);
  console.log(`unmatched: ${unmatched.join(', ')}`);
  console.log(`pngs: ${pngNote}`);
}

main();
