import { createHash } from "node:crypto";
import { error as logError, log } from "node:console";
import { mkdir, readFile, readdir, writeFile } from "node:fs/promises";
import { basename, dirname, join, relative } from "node:path";
import process from "node:process";
import { fileURLToPath, URL } from "node:url";

const repositoryRoot = fileURLToPath(new URL("../..", import.meta.url));
const schemaDirectory = join(repositoryRoot, "packages", "contracts", "schemas");
const checkMode = process.argv.slice(2).includes("--check");
const unexpectedArguments = process.argv.slice(2).filter((argument) => argument !== "--check");

const outputDefinitions = [
  {
    extension: ".ts",
    language: "typescript",
    outputDirectory: join(repositoryRoot, "packages", "contracts", "src", "generated"),
  },
  {
    extension: ".py",
    language: "python",
    outputDirectory: join(repositoryRoot, "engine", "teratai_engine", "generated"),
  },
  {
    extension: ".rs",
    language: "rust",
    outputDirectory: join(repositoryRoot, "packages", "contracts", "rust", "src", "generated"),
  },
];

function assertCondition(condition, message) {
  if (!condition) throw new Error(message);
}

function isRecord(value) {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function compareAscii(left, right) {
  if (left < right) return -1;
  if (left > right) return 1;
  return 0;
}

function normalizeDescription(value) {
  return value.replaceAll("\r", " ").replaceAll("\n", " ").replaceAll("*/", "* /").trim();
}

function schemaBaseName(schemaPath) {
  return basename(schemaPath, ".schema.json");
}

function snakeCaseFileName(schemaPath) {
  return schemaBaseName(schemaPath).replaceAll("-", "_");
}

function validateProperty(property, propertyPath) {
  assertCondition(isRecord(property), `${propertyPath} must be an object.`);
  assertCondition(
    typeof property.description === "string" && property.description.trim().length > 0,
    `${propertyPath}.description must be a non-empty string.`,
  );
  assertCondition(
    ["array", "boolean", "integer", "number", "string"].includes(property.type),
    `${propertyPath}.type is not supported by generator revision 1.`,
  );

  if (property.type === "array") {
    assertCondition(isRecord(property.items), `${propertyPath}.items must be an object.`);
    validateProperty(
      { description: `${property.description} item`, ...property.items },
      `${propertyPath}.items`,
    );
    assertCondition(property.items.type !== "array", `${propertyPath} cannot contain nested arrays.`);
  }
}

function validateSchema(schema, schemaPath) {
  const displayPath = relative(repositoryRoot, schemaPath);
  assertCondition(isRecord(schema), `${displayPath} must contain a JSON object.`);
  assertCondition(
    schema.$schema === "https://json-schema.org/draft/2020-12/schema",
    `${displayPath} must use JSON Schema draft 2020-12.`,
  );
  assertCondition(typeof schema.$id === "string" && schema.$id.length > 0, `${displayPath} needs $id.`);
  assertCondition(
    typeof schema.title === "string" && /^[A-Z][A-Za-z0-9]*$/.test(schema.title),
    `${displayPath}.title must be a PascalCase identifier.`,
  );
  assertCondition(
    typeof schema.description === "string" && schema.description.trim().length > 0,
    `${displayPath}.description must be a non-empty string.`,
  );
  assertCondition(!schema.description.includes('"""'), `${displayPath}.description cannot contain triple quotes.`);
  assertCondition(schema.type === "object", `${displayPath} root type must be object.`);
  assertCondition(
    schema.additionalProperties === true,
    `${displayPath} must tolerate additive fields for protocol compatibility.`,
  );
  assertCondition(isRecord(schema.properties), `${displayPath}.properties must be an object.`);
  assertCondition(Array.isArray(schema.required), `${displayPath}.required must be an array.`);

  const propertyNames = Object.keys(schema.properties);
  const requiredNames = schema.required;
  assertCondition(propertyNames.length > 0, `${displayPath} must define properties.`);
  assertCondition(
    requiredNames.every((name) => typeof name === "string"),
    `${displayPath}.required must contain only strings.`,
  );
  assertCondition(
    new Set(requiredNames).size === requiredNames.length,
    `${displayPath}.required cannot contain duplicates.`,
  );

  for (const propertyName of propertyNames) {
    assertCondition(
      /^[a-z][a-z0-9_]*$/.test(propertyName),
      `${displayPath}.properties.${propertyName} must be a snake_case identifier.`,
    );
    validateProperty(schema.properties[propertyName], `${displayPath}.properties.${propertyName}`);
  }

  for (const requiredName of requiredNames) {
    assertCondition(
      Object.hasOwn(schema.properties, requiredName),
      `${displayPath}.required references unknown property ${requiredName}.`,
    );
  }
}

function orderedProperties(schema) {
  const required = new Set(schema.required);
  return Object.entries(schema.properties).sort(([left], [right]) => {
    const requiredOrder = Number(required.has(right)) - Number(required.has(left));
    return requiredOrder === 0 ? compareAscii(left, right) : requiredOrder;
  });
}

function mapType(property, language) {
  const primitives = {
    python: { boolean: "bool", integer: "int", number: "float", string: "str" },
    rust: { boolean: "bool", integer: "i64", number: "f64", string: "String" },
    typescript: { boolean: "boolean", integer: "number", number: "number", string: "string" },
  };

  if (property.type === "array") {
    const itemType = mapType(property.items, language);
    if (language === "python") return `list[${itemType}]`;
    if (language === "rust") return `Vec<${itemType}>`;
    return `readonly ${itemType}[]`;
  }

  return primitives[language][property.type];
}

function renderTypeScript(schema, sourcePath, fingerprint) {
  const required = new Set(schema.required);
  const properties = orderedProperties(schema).map(([name, property]) => {
    const optionalMarker = required.has(name) ? "" : "?";
    return [
      `  /** ${normalizeDescription(property.description)} */`,
      `  readonly ${name}${optionalMarker}: ${mapType(property, "typescript")};`,
    ].join("\n");
  });

  return [
    `// Generated from ${sourcePath}.`,
    `// Schema SHA-256: ${fingerprint}.`,
    "// Do not edit manually.",
    "",
    `/** ${normalizeDescription(schema.description)} */`,
    `export interface ${schema.title} {`,
    ...properties,
    "  readonly [additionalProperty: string]: unknown;",
    "}",
    "",
  ].join("\n");
}

function renderPython(schema, sourcePath, fingerprint) {
  const required = new Set(schema.required);
  const properties = orderedProperties(schema).map(([name, property]) => {
    const mappedType = mapType(property, "python");
    const type = required.has(name) ? mappedType : `${mappedType} | None`;
    const defaultValue = required.has(name) ? "" : " = None";
    return `    ${name}: ${type}${defaultValue}`;
  });

  return [
    `# Generated from ${sourcePath}.`,
    `# Schema SHA-256: ${fingerprint}.`,
    "# Do not edit manually.",
    "from __future__ import annotations",
    "",
    "from dataclasses import dataclass",
    "",
    "",
    "@dataclass(frozen=True, slots=True)",
    `class ${schema.title}:`,
    `    """${normalizeDescription(schema.description)}"""`,
    "",
    ...properties,
    "",
  ].join("\n");
}

function renderRust(schema, sourcePath, fingerprint) {
  const required = new Set(schema.required);
  const properties = orderedProperties(schema).flatMap(([name, property]) => {
    const mappedType = mapType(property, "rust");
    const type = required.has(name) ? mappedType : `Option<${mappedType}>`;
    return [`    /// ${normalizeDescription(property.description)}`, `    pub ${name}: ${type},`];
  });

  return [
    `// Generated from ${sourcePath}.`,
    `// Schema SHA-256: ${fingerprint}.`,
    "// Do not edit manually.",
    "",
    `/// ${normalizeDescription(schema.description)}`,
    "#[derive(Debug, Clone, PartialEq, serde::Deserialize, serde::Serialize)]",
    `pub struct ${schema.title} {`,
    ...properties,
    "}",
    "",
  ].join("\n");
}

function renderOutput(language, schema, sourcePath, fingerprint) {
  if (language === "typescript") return renderTypeScript(schema, sourcePath, fingerprint);
  if (language === "python") return renderPython(schema, sourcePath, fingerprint);
  if (language === "rust") return renderRust(schema, sourcePath, fingerprint);
  throw new Error(`Unsupported output language: ${language}.`);
}

async function readExisting(path) {
  try {
    return await readFile(path, "utf8");
  } catch (error) {
    if (isRecord(error) && error.code === "ENOENT") return undefined;
    throw error;
  }
}

async function main() {
  assertCondition(unexpectedArguments.length === 0, `Unknown arguments: ${unexpectedArguments.join(", ")}`);
  const schemaFileNames = (await readdir(schemaDirectory))
    .filter((fileName) => fileName.endsWith(".schema.json"))
    .sort(compareAscii);
  assertCondition(schemaFileNames.length > 0, "No canonical contract schemas were found.");

  const driftedFiles = [];
  const generatedModules = [];
  for (const schemaFileName of schemaFileNames) {
    const schemaPath = join(schemaDirectory, schemaFileName);
    const schemaSource = await readFile(schemaPath, "utf8");
    const schema = JSON.parse(schemaSource);
    validateSchema(schema, schemaPath);
    const sourcePath = relative(repositoryRoot, schemaPath).replaceAll("\\", "/");
    const fingerprint = createHash("sha256").update(schemaSource).digest("hex");
    generatedModules.push(snakeCaseFileName(schemaPath));

    for (const outputDefinition of outputDefinitions) {
      const outputName = outputDefinition.language === "typescript"
        ? schemaBaseName(schemaPath)
        : snakeCaseFileName(schemaPath);
      const outputPath = join(outputDefinition.outputDirectory, `${outputName}${outputDefinition.extension}`);
      const expected = renderOutput(outputDefinition.language, schema, sourcePath, fingerprint);
      const existing = await readExisting(outputPath);

      if (existing === expected) continue;
      if (checkMode) {
        driftedFiles.push(relative(repositoryRoot, outputPath).replaceAll("\\", "/"));
        continue;
      }

      await mkdir(dirname(outputPath), { recursive: true });
      await writeFile(outputPath, expected, "utf8");
      log(`Generated ${relative(repositoryRoot, outputPath)}`);
    }
  }

  const rustModulePath = join(repositoryRoot, "packages", "contracts", "rust", "src", "generated", "mod.rs");
  const expectedRustModules = [
    "//! Types generated from canonical cross-language contract schemas.",
    "",
    ...generatedModules.map((moduleName) => `pub mod ${moduleName};`),
    "",
  ].join("\n");
  const existingRustModules = await readExisting(rustModulePath);
  if (existingRustModules !== expectedRustModules) {
    if (checkMode) driftedFiles.push(relative(repositoryRoot, rustModulePath).replaceAll("\\", "/"));
    else {
      await mkdir(dirname(rustModulePath), { recursive: true });
      await writeFile(rustModulePath, expectedRustModules, "utf8");
      log(`Generated ${relative(repositoryRoot, rustModulePath)}`);
    }
  }

  if (driftedFiles.length > 0) {
    throw new Error(
      `Generated contracts are missing or stale:\n${driftedFiles.map((path) => `- ${path}`).join("\n")}\nRun pnpm contracts:generate.`,
    );
  }

  if (checkMode) log(`Verified ${schemaFileNames.length} canonical contract schema(s).`);
}

main().catch((error) => {
  logError(error instanceof Error ? error.message : String(error));
  process.exitCode = 1;
});
