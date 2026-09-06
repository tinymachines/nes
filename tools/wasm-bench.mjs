// The console under node as wasm (N8 step 3): frames a second with the
// sound on, and a sanity check on what comes out.
//   wasm-pack build crates/nes-wasm --target nodejs --out-dir /tmp/nes-wasm --release
//   node tools/wasm-bench.mjs /tmp/nes-wasm rom.nes [frames]
import { createRequire } from "node:module";
import { readFileSync } from "node:fs";
const require = createRequire(import.meta.url);
const [, , pkg, rom, framesArg] = process.argv;
if (!pkg || !rom) {
  console.error("usage: node tools/wasm-bench.mjs <pkg dir> rom.nes [frames]");
  process.exit(2);
}
const { Nes } = require(`${process.cwd()}/${pkg}/nes_wasm.js`.replace(`${process.cwd()}//`, "/"));
const frames = Number(framesArg ?? 300);
const nes = new Nes(readFileSync(rom));
nes.run_frames(30);
nes.sound();
const t0 = performance.now();
nes.run_frames(frames);
const dt = (performance.now() - t0) / 1000;
const colour = nes.colour();
const sound = nes.sound();
const distinct = new Set(colour).size;
console.log(
  `wasm: ${frames} frames in ${dt.toFixed(2)} s (${(frames / dt).toFixed(1)} frames/s, ${(frames / dt / 60.0988).toFixed(2)}x real time); ` +
    `frame ${colour.length} dots, ${distinct} distinct colours, parity ${nes.parity()}; ${sound.length} sound samples (${(sound.length / frames).toFixed(1)} a frame)`
);
