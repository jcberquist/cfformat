// Reads a JSON array of cases from stdin:
//   [{ doc, printWidth, tabWidth, useTabs, endOfLine }, ...]
// and writes a JSON array with printDocToString(doc, options).formatted for
// each. `doc` is the shape written by cfdoc::debug::to_prettier_json.
import { printer } from "prettier/doc";

function revive(key, value) {
  if (value && typeof value === "object" && !Array.isArray(value)) {
    // dedentToRoot: -Infinity has no JSON form.
    if (value.type === "align" && value.n === null) {
      value.n = Number.NEGATIVE_INFINITY;
    }
    // Prettier's conditionalGroup shares states[0] as contents.
    if (value.type === "group" && value.expandedStates) {
      value.contents = value.expandedStates[0];
    }
  }
  return value;
}

let input = "";
for await (const chunk of process.stdin) {
  input += chunk;
}

const cases = JSON.parse(input, revive);
const output = cases.map(({ doc, ...options }) =>
  printer.printDocToString(doc, options).formatted,
);
process.stdout.write(JSON.stringify(output));
