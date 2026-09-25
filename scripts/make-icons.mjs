import sharp from "sharp";
import pngToIco from "png-to-ico";
import { mkdir, writeFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";

const source = new URL("../src-tauri/icons/icon.svg", import.meta.url);
const output = new URL("../src-tauri/icons/", import.meta.url);
await mkdir(output, { recursive: true });
const sizes = [32, 64, 128, 256];
const buffers = [];
for (const size of sizes) {
  const buffer = await sharp(fileURLToPath(source)).resize(size, size).png().toBuffer();
  buffers.push(buffer);
  if (size === 32) await writeFile(new URL("32x32.png", output), buffer);
  if (size === 128) await writeFile(new URL("128x128.png", output), buffer);
  if (size === 256) await writeFile(new URL("128x128@2x.png", output), buffer);
}
await writeFile(new URL("icon.png", output), buffers.at(-1));
await writeFile(new URL("icon.ico", output), await pngToIco(buffers));
