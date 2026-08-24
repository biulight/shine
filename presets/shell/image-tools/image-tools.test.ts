import { afterEach, describe, expect, test } from "bun:test";
import { mkdtemp, mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import {
  expandImageInputs,
  outputPathFor,
  parseImageToolOptions,
  planImageTasks,
  runImageTool,
  writeFailureLog,
  type ImageFailure,
} from "./image-tools.ts";

const temporaryDirectories: string[] = [];
const PNG_1X1 = Uint8Array.from(
  Buffer.from(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=",
    "base64",
  ),
);

async function temporaryDirectory(): Promise<string> {
  const directory = await mkdtemp(join(tmpdir(), "shine-image-tools-"));
  temporaryDirectories.push(directory);
  return directory;
}

afterEach(async () => {
  await Promise.all(
    temporaryDirectories.splice(0).map((directory) =>
      rm(directory, {
        recursive: true,
        force: true,
      }),
    ),
  );
});

describe("image tool options", () => {
  test("uses configured defaults and lets command arguments override them", () => {
    const defaults = parseImageToolOptions("resize", ["photo.jpg"], {
      IMAGE_QUALITY: "75",
      IMAGE_MAX_WIDTH: "1600",
      IMAGE_MAX_HEIGHT: "900",
    });
    expect(defaults).toMatchObject({ quality: 75, width: 1600, height: 900 });

    const overridden = parseImageToolOptions(
      "resize",
      ["--quality", "90", "--width", "800", "--height", "600", "photo.jpg"],
      {},
    );
    expect(overridden).toMatchObject({ quality: 90, width: 800, height: 600 });
  });

  test("validates operation-specific arguments", () => {
    expect(() =>
      parseImageToolOptions("compress", ["--width", "10", "photo.jpg"], {}),
    ).toThrow("only valid for img-resize");
    expect(() => parseImageToolOptions("convert", ["photo.jpg"], {})).toThrow(
      "requires --format",
    );
    expect(() =>
      parseImageToolOptions("compress", ["--quality", "101", "photo.jpg"], {}),
    ).toThrow("between 1 and 100");
  });
});

describe("input discovery and output planning", () => {
  test("scans one directory level and ignores nested or unsupported entries", async () => {
    const directory = await temporaryDirectory();
    await writeFile(join(directory, "first.JPG"), PNG_1X1);
    await writeFile(join(directory, "notes.txt"), "not an image");
    await mkdir(join(directory, "nested"));
    await writeFile(join(directory, "nested", "second.png"), PNG_1X1);

    const expanded = await expandImageInputs([directory]);
    expect(expanded.files).toEqual([join(directory, "first.JPG")]);
    expect(expanded.failures).toEqual([]);
    expect(expanded.ignored).toBe(2);
  });

  test("derives safe names and rejects flattened output collisions", () => {
    const output = resolve("output");
    const first = join(resolve("one"), "photo.jpg");
    const second = join(resolve("two"), "photo.jpg");
    const options = parseImageToolOptions(
      "compress",
      ["--output-dir", output, first, second],
      {},
    );
    expect(outputPathFor(join(resolve("one"), "photo.jpeg"), options)).toBe(
      join(output, "photo.compressed.jpg"),
    );
    const plan = planImageTasks([first, second], options);
    expect(plan.tasks).toEqual([]);
    expect(plan.failures).toHaveLength(2);
  });

  test("never maps one converted image over another input", () => {
    const directory = resolve("images");
    const webp = join(directory, "photo.webp");
    const jpeg = join(directory, "photo.jpg");
    const options = parseImageToolOptions(
      "convert",
      ["--format", "jpg", webp, jpeg],
      {},
    );
    const plan = planImageTasks([webp, jpeg], options);
    expect(plan.tasks).toEqual([]);
    expect(plan.failures).toHaveLength(2);
    expect(plan.failures.every((failure) => failure.message.includes("multiple inputs map")))
      .toBe(true);
  });
});

describe("image processing", () => {
  test("converts an image and refuses to replace it without --force", async () => {
    const directory = await temporaryDirectory();
    const output = join(directory, "out");
    const input = join(directory, "pixel.png");
    await writeFile(input, PNG_1X1);

    expect(
      await runImageTool("convert", [
        "--format",
        "webp",
        "--output-dir",
        output,
        input,
      ]),
    ).toBe(0);
    const converted = join(output, "pixel.webp");
    expect((await new Bun.Image(converted).metadata()).format).toBe("webp");
    const firstBytes = await readFile(converted);

    expect(
      await runImageTool("convert", [
        "--format",
        "webp",
        "--output-dir",
        output,
        input,
      ]),
    ).toBe(1);
    expect(await readFile(converted)).toEqual(firstBytes);

    expect(
      await runImageTool("convert", [
        "--format",
        "webp",
        "--output-dir",
        output,
        "--force",
        input,
      ]),
    ).toBe(0);
  });

  test("skips resizing an image already inside the configured bounds", async () => {
    const directory = await temporaryDirectory();
    const input = join(directory, "pixel.png");
    await writeFile(input, PNG_1X1);
    expect(
      await runImageTool("resize", ["--width", "10", "--height", "10", input]),
    ).toBe(0);
    expect(await Bun.file(join(directory, "pixel.resized.png")).exists()).toBe(false);
  });
});

test("writes a complete failure log only after twenty failures", async () => {
  const directory = await temporaryDirectory();
  const failures: ImageFailure[] = Array.from({ length: 21 }, (_, index) => ({
    input: `/input/${index}.jpg`,
    message: `failure ${index}`,
  }));
  expect(await writeFailureLog(failures.slice(0, 20), directory)).toBeUndefined();
  const log = await writeFailureLog(
    failures,
    directory,
    new Date("2026-08-23T12:34:56Z"),
  );
  expect(log).toBe(join(directory, "image-tools-errors-20260823-123456Z.log"));
  expect(await readFile(log!, "utf8")).toContain("/input/20.jpg: failure 20");
});
