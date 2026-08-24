import { randomUUID } from "node:crypto";
import {
  link,
  lstat,
  mkdir,
  readdir,
  rename,
  unlink,
  writeFile,
} from "node:fs/promises";
import { basename, dirname, extname, join, parse, resolve } from "node:path";
import { parseArgs } from "node:util";

export type ImageOperation = "compress" | "resize" | "convert";
export type PortableFormat = "jpeg" | "png" | "webp";

const SUPPORTED_EXTENSIONS = new Set([".jpg", ".jpeg", ".png", ".webp"]);
const FAILURE_LOG_THRESHOLD = 20;
const DEFAULT_QUALITY = 80;
const DEFAULT_MAX_WIDTH = 1920;
const DEFAULT_MAX_HEIGHT = 1080;

export interface ImageToolOptions {
  operation: ImageOperation;
  inputs: string[];
  outputDir?: string;
  force: boolean;
  quality: number;
  width: number;
  height: number;
  format?: PortableFormat;
  help: boolean;
}

export interface ImageFailure {
  input: string;
  message: string;
}

interface ImageTask {
  input: string;
  output: string;
}

interface ExpandedInputs {
  files: string[];
  failures: ImageFailure[];
  ignored: number;
}

function positiveInteger(name: string, value: string | undefined, fallback: number): number {
  const parsed = Number(value ?? fallback);
  if (!Number.isSafeInteger(parsed) || parsed <= 0) {
    throw new Error(`${name} must be a positive integer`);
  }
  return parsed;
}

function qualityValue(value: string | undefined): number {
  const quality = positiveInteger("quality", value, DEFAULT_QUALITY);
  if (quality > 100) {
    throw new Error("quality must be between 1 and 100");
  }
  return quality;
}

function portableFormat(value: string | undefined): PortableFormat | undefined {
  if (value === undefined) return undefined;
  const normalized = value.toLowerCase() === "jpg" ? "jpeg" : value.toLowerCase();
  if (normalized !== "jpeg" && normalized !== "png" && normalized !== "webp") {
    throw new Error("format must be one of: jpg, jpeg, png, webp");
  }
  return normalized;
}

export function parseImageToolOptions(
  operation: ImageOperation,
  args: readonly string[],
  environment: Readonly<Record<string, string | undefined>> = Bun.env,
): ImageToolOptions {
  const options = {
    "output-dir": { type: "string" as const },
    force: { type: "boolean" as const, default: false },
    quality: { type: "string" as const },
    width: { type: "string" as const },
    height: { type: "string" as const },
    format: { type: "string" as const },
    help: { type: "boolean" as const, default: false },
  };
  const parsed = parseArgs({
    args: [...args],
    allowPositionals: true,
    strict: true,
    options,
  });
  const help = parsed.values.help ?? false;
  if (!help && parsed.positionals.length === 0) {
    throw new Error("at least one input file or directory is required");
  }
  if (operation !== "resize" && (parsed.values.width || parsed.values.height)) {
    throw new Error("--width and --height are only valid for img-resize");
  }
  if (operation !== "convert" && parsed.values.format) {
    throw new Error("--format is only valid for img-convert");
  }
  const format = portableFormat(parsed.values.format);
  if (!help && operation === "convert" && !format) {
    throw new Error("img-convert requires --format <jpg|jpeg|png|webp>");
  }

  return {
    operation,
    inputs: parsed.positionals,
    outputDir: parsed.values["output-dir"],
    force: parsed.values.force ?? false,
    quality: qualityValue(
      help ? undefined : (parsed.values.quality ?? environment.IMAGE_QUALITY),
    ),
    width: positiveInteger(
      "width",
      help ? undefined : (parsed.values.width ?? environment.IMAGE_MAX_WIDTH),
      DEFAULT_MAX_WIDTH,
    ),
    height: positiveInteger(
      "height",
      help ? undefined : (parsed.values.height ?? environment.IMAGE_MAX_HEIGHT),
      DEFAULT_MAX_HEIGHT,
    ),
    format,
    help,
  };
}

function usage(operation: ImageOperation): string {
  const common = "[--output-dir DIR] [--force] <INPUT...>";
  switch (operation) {
    case "compress":
      return `Usage: img-compress [--quality 1..100] ${common}`;
    case "resize":
      return `Usage: img-resize [--width PX] [--height PX] [--quality 1..100] ${common}`;
    case "convert":
      return `Usage: img-convert --format <jpg|jpeg|png|webp> [--quality 1..100] ${common}`;
  }
}

function isSupportedPath(path: string): boolean {
  return SUPPORTED_EXTENSIONS.has(extname(path).toLowerCase());
}

export async function expandImageInputs(inputs: readonly string[]): Promise<ExpandedInputs> {
  const files = new Map<string, string>();
  const failures: ImageFailure[] = [];
  let ignored = 0;

  for (const input of inputs) {
    const absolute = resolve(input);
    let stat;
    try {
      stat = await lstat(absolute);
    } catch (error) {
      failures.push({ input: absolute, message: errorMessage(error) });
      continue;
    }
    if (stat.isSymbolicLink()) {
      failures.push({ input: absolute, message: "symbolic-link inputs are not supported" });
      continue;
    }
    if (stat.isFile()) {
      if (!isSupportedPath(absolute)) {
        failures.push({ input: absolute, message: "expected a JPEG, PNG, or WebP file" });
      } else if (files.has(absolute)) {
        ignored += 1;
      } else {
        files.set(absolute, absolute);
      }
      continue;
    }
    if (!stat.isDirectory()) {
      failures.push({ input: absolute, message: "input is not a regular file or directory" });
      continue;
    }

    let found = 0;
    try {
      for (const entry of await readdir(absolute, { withFileTypes: true })) {
        if (!entry.isFile() || !isSupportedPath(entry.name)) {
          ignored += 1;
          continue;
        }
        const candidate = join(absolute, entry.name);
        found += 1;
        if (files.has(candidate)) ignored += 1;
        else files.set(candidate, candidate);
      }
    } catch (error) {
      failures.push({ input: absolute, message: errorMessage(error) });
      continue;
    }
    if (found === 0) {
      failures.push({ input: absolute, message: "directory contains no JPEG, PNG, or WebP files" });
    }
  }

  return { files: [...files.values()], failures, ignored };
}

function normalizedSourceExtension(path: string): string {
  const extension = extname(path).toLowerCase();
  return extension === ".jpeg" ? ".jpg" : extension;
}

export function outputPathFor(input: string, options: ImageToolOptions): string {
  const parsed = parse(input);
  const targetDirectory = options.outputDir ? resolve(options.outputDir) : parsed.dir;
  switch (options.operation) {
    case "compress":
      return join(targetDirectory, `${parsed.name}.compressed${normalizedSourceExtension(input)}`);
    case "resize":
      return join(targetDirectory, `${parsed.name}.resized${normalizedSourceExtension(input)}`);
    case "convert": {
      const extension = options.format === "jpeg" ? ".jpg" : `.${options.format}`;
      return join(targetDirectory, `${parsed.name}${extension}`);
    }
  }
}

export function planImageTasks(
  files: readonly string[],
  options: ImageToolOptions,
): { tasks: ImageTask[]; failures: ImageFailure[] } {
  const byOutput = new Map<string, ImageTask[]>();
  for (const input of files) {
    const task = { input, output: outputPathFor(input, options) };
    const bucket = byOutput.get(task.output) ?? [];
    bucket.push(task);
    byOutput.set(task.output, bucket);
  }
  const tasks: ImageTask[] = [];
  const failures: ImageFailure[] = [];
  const sourcePaths = new Set(files.map((file) => resolve(file)));
  for (const [output, candidates] of byOutput) {
    if (candidates.length === 1) {
      const candidate = candidates[0]!;
      if (sourcePaths.has(resolve(output)) && resolve(output) !== resolve(candidate.input)) {
        failures.push({
          input: candidate.input,
          message: `output would replace another input: ${output}`,
        });
      } else {
        tasks.push(candidate);
      }
      continue;
    }
    for (const candidate of candidates) {
      failures.push({ input: candidate.input, message: `multiple inputs map to ${output}` });
    }
  }
  return { tasks, failures };
}

function assertPortableSource(metadata: Bun.Image.Metadata): PortableFormat {
  if (metadata.format !== "jpeg" && metadata.format !== "png" && metadata.format !== "webp") {
    throw new Error(`unsupported decoded format: ${metadata.format}`);
  }
  return metadata.format;
}

function configureFormat(
  pipeline: Bun.Image,
  format: PortableFormat,
  quality: number,
): Bun.Image {
  switch (format) {
    case "jpeg":
      return pipeline.jpeg({ quality, progressive: true });
    case "png":
      return pipeline.png({ compressionLevel: 9 });
    case "webp":
      return pipeline.webp({ quality });
  }
}

async function pathExists(path: string): Promise<boolean> {
  try {
    await lstat(path);
    return true;
  } catch (error) {
    if (isErrorCode(error, "ENOENT")) return false;
    throw error;
  }
}

async function writeOutput(path: string, bytes: Uint8Array, force: boolean): Promise<void> {
  await mkdir(dirname(path), { recursive: true });
  if (!force && (await pathExists(path))) {
    throw new Error("output already exists; pass --force to replace it");
  }
  const temporary = join(
    dirname(path),
    `.${basename(path)}.shine-${process.pid}-${randomUUID()}.tmp`,
  );
  try {
    await writeFile(temporary, bytes, { flag: "wx" });
    if (force) {
      await rename(temporary, path);
    } else {
      await link(temporary, path);
      await unlink(temporary);
    }
  } catch (error) {
    try {
      await unlink(temporary);
    } catch (cleanupError) {
      if (!isErrorCode(cleanupError, "ENOENT")) {
        console.error(
          `img-tools: could not remove temporary file ${temporary}: ${errorMessage(cleanupError)}`,
        );
      }
    }
    if (!force && isErrorCode(error, "EEXIST")) {
      throw new Error("output already exists; pass --force to replace it");
    }
    throw error;
  }
}

async function processTask(
  task: ImageTask,
  options: ImageToolOptions,
): Promise<"written" | "skipped"> {
  const metadata = await new Bun.Image(task.input).metadata();
  const sourceFormat = assertPortableSource(metadata);
  if (
    options.operation === "resize" &&
    metadata.width <= options.width &&
    metadata.height <= options.height
  ) {
    return "skipped";
  }
  if (options.operation === "convert" && sourceFormat === options.format) {
    return "skipped";
  }
  if (resolve(task.input) === resolve(task.output)) {
    throw new Error("output would replace the input file");
  }

  let pipeline = new Bun.Image(task.input);
  if (options.operation === "resize") {
    pipeline = pipeline.resize(options.width, options.height, {
      fit: "inside",
      withoutEnlargement: true,
    });
  }
  const outputFormat = options.operation === "convert" ? options.format! : sourceFormat;
  const bytes = await configureFormat(pipeline, outputFormat, options.quality).bytes();
  await writeOutput(task.output, bytes, options.force);
  return "written";
}

function timestamp(date: Date): string {
  return date
    .toISOString()
    .replace(/[-:]/g, "")
    .replace(/\.\d{3}Z$/, "Z")
    .replace("T", "-");
}

export async function writeFailureLog(
  failures: readonly ImageFailure[],
  directory: string,
  now: Date = new Date(),
): Promise<string | undefined> {
  if (failures.length <= FAILURE_LOG_THRESHOLD) return undefined;
  await mkdir(directory, { recursive: true });
  const body =
    failures.map((failure) => `${failure.input}: ${failure.message}`).join("\n") + "\n";
  const stem = `image-tools-errors-${timestamp(now)}`;
  for (let suffix = 0; ; suffix += 1) {
    const path = join(directory, `${stem}${suffix === 0 ? "" : `-${suffix}`}.log`);
    try {
      await writeFile(path, body, { flag: "wx" });
      return path;
    } catch (error) {
      if (!isErrorCode(error, "EEXIST")) throw error;
    }
  }
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function isErrorCode(error: unknown, code: string): boolean {
  return (
    typeof error === "object" &&
    error !== null &&
    "code" in error &&
    error.code === code
  );
}

function printFailures(failures: readonly ImageFailure[]): void {
  const visible = failures.length > FAILURE_LOG_THRESHOLD
    ? failures.slice(0, FAILURE_LOG_THRESHOLD)
    : failures;
  for (const failure of visible) {
    console.error(`failed: ${failure.input}: ${failure.message}`);
  }
}

export async function runImageTool(
  operation: ImageOperation,
  args: readonly string[] = Bun.argv.slice(2),
): Promise<number> {
  const command = `img-${operation}`;
  try {
    const options = parseImageToolOptions(operation, args);
    if (options.help) {
      console.log(usage(operation));
      console.log("Inputs may be files or directories; directories are scanned one level only.");
      return 0;
    }
    if (typeof Bun.Image !== "function") {
      throw new Error("Bun.Image is unavailable; install Bun 1.3.14 or newer");
    }

    const expanded = await expandImageInputs(options.inputs);
    const planned = planImageTasks(expanded.files, options);
    const failures = [...expanded.failures, ...planned.failures];
    let written = 0;
    let skipped = expanded.ignored;

    for (const task of planned.tasks) {
      try {
        const outcome = await processTask(task, options);
        if (outcome === "written") {
          written += 1;
          console.log(`${task.input} -> ${task.output}`);
        } else {
          skipped += 1;
          console.log(`skipped: ${task.input}`);
        }
      } catch (error) {
        failures.push({ input: task.input, message: errorMessage(error) });
      }
    }

    printFailures(failures);
    let failureLog: string | undefined;
    try {
      failureLog = await writeFailureLog(
        failures,
        options.outputDir ? resolve(options.outputDir) : process.cwd(),
      );
    } catch (error) {
      failures.push({ input: "failure log", message: errorMessage(error) });
      console.error(`failed: could not write complete failure log: ${errorMessage(error)}`);
    }
    if (failureLog) console.error(`complete failure log: ${failureLog}`);
    console.log(`summary: ${written} written, ${skipped} skipped, ${failures.length} failed`);
    return failures.length === 0 ? 0 : 1;
  } catch (error) {
    console.error(`${command}: ${errorMessage(error)}`);
    console.error(usage(operation));
    return 1;
  }
}
