
# Problem Statements for Building a Rust-Based Tailwind Linter

Here are "LeetCode-style" problem statements that define the key challenges you'll need to solve when building your Tailwind linter in Rust:

## Problem 1: TypeScript/React File Parser
**Difficulty: Medium**

**Problem Statement:**  
Create a Rust function that accepts a file path as input and parses a TypeScript/React (.tsx) file into an Abstract Syntax Tree (AST). The function should handle all valid TSX syntax constructs including JSX elements, TypeScript types, and modern JavaScript features.

**Input:**  
A string containing the path to a .tsx file.

**Output:**  
An AST representation of the file contents.

**Constraints:**
- Must handle valid TypeScript and JSX syntax
- Must be capable of processing files up to 1MB in size
- Should report meaningful errors for invalid syntax

**Example:**
```
Input: "./components/Button.tsx"
Output: [AST representation of the Button component]
```

## Problem 2: JSX Attribute Extractor
**Difficulty: Medium**

**Problem Statement:**  
Given an AST from a TypeScript/React file, extract all JSX elements and their `className` attributes. Handle both static string literals and dynamic expressions.

**Input:**  
An AST of a TypeScript/React file.

**Output:**  
A collection of objects containing:
- File path
- Line and column number of each className attribute
- The raw value of the className attribute (string literal or expression)
- For string literals, the parsed string value

**Constraints:**
- Must handle string literals: `className="..."` 
- Must identify (but might not fully evaluate) dynamic expressions: `className={...}`
- Must handle template literals with and without expressions
- Must handle className prop spreading: `{...props}`

**Example:**
```
Input: AST for `<div className="text-red-500 flex p-4">Hello</div>`
Output: {
path: "./components/Button.tsx",
          line: 5,
          column: 10,
          rawValue: "text-red-500 flex p-4",
          stringValue: "text-red-500 flex p-4"
}
```

## Problem 3: Tailwind Class Parser
**Difficulty: Medium**

**Problem Statement:**  
Given a string containing Tailwind CSS classes, parse it into individual class tokens and categorize them by their utility type (spacing, color, layout, etc.).

**Input:**  
A string containing space-separated Tailwind CSS classes.

**Output:**  
A structured representation of the Tailwind classes, categorized by utility type.

**Constraints:**
- Must handle core Tailwind utility classes
- Must handle responsive prefixes (sm:, md:, lg:, etc.)
- Must handle state variants (hover:, focus:, etc.)
- Must handle arbitrary values (e.g., `w-[300px]`)

**Example:**
```
Input: "flex p-4 text-red-500 sm:text-blue-700 hover:bg-gray-100"
Output: {
        layout: ["flex"],
        spacing: ["p-4"],
        typography: ["text-red-500", "sm:text-blue-700"],
        background: ["hover:bg-gray-100"]
}
```

## Problem 4: Tailwind Conflict Detector
**Difficulty: Hard**

**Problem Statement:**  
Given a set of categorized Tailwind classes, detect conflicts where multiple classes from the same utility category might override each other unexpectedly.

**Input:**  
A structured representation of Tailwind classes categorized by utility type.

**Output:**  
A list of detected conflicts, including the conflicting classes and their locations.

**Constraints:**
- Must detect direct conflicts (e.g., `text-red-500 text-blue-500`)
- Must consider responsive variants (e.g., `text-red-500 sm:text-red-500` is not a conflict)
- Must consider state variants (e.g., `text-red-500 hover:text-red-500` is not a conflict)
- Should ignore intentional overrides in the expected responsive/variant order

**Example:**
```
Input: {
      typography: ["text-red-500", "text-blue-700"]
}
Output: [
{
conflict: "text-color",
              classes: ["text-red-500", "text-blue-700"],
              line: 5,
              column: 10
}
]
                        ```

## Problem 5: Rule Engine
**Difficulty: Medium**

**Problem Statement:**  
Create a rule engine that can apply configurable linting rules to Tailwind class sets.

**Input:**  
1. A structured representation of Tailwind classes
2. A configuration specifying which rules to apply and their settings

**Output:**  
A list of rule violations with detailed information for each.

**Constraints:**
- Must support enabling/disabling individual rules
- Must support rule severity levels (error, warning, info)
- Rules should be pluggable via a standard interface
- Should allow custom rules to be added easily

**Example:**
```
Input: 
- Classes: {layout: ["flex"], spacing: ["p-4", "p-6"]}
- Config: {rules: {conflictingClasses: "error", preferredSpacing: {enabled: true, values: [4, 8]}}}

Output: [
  {
          rule: "conflictingClasses",
              message: "Conflicting padding classes: p-4 and p-6",
                  severity: "error",
                      line: 5,
                          column: 10
                            },
                              {
                                      rule: "preferredSpacing",
                                          message: "p-6 uses non-preferred spacing value. Use p-4 or p-8 instead.",
                                              severity: "warning",
                                                  line: 5,
                                                      column: 15
                                                        }
                                                        ]
                                                        ```

## Problem 6: Diagnostic Reporter
**Difficulty: Easy**

**Problem Statement:**  
Create a formatter that converts rule violations into compiler-style diagnostic messages that Vim's quickfix feature can parse.

**Input:**  
A list of rule violations from the rule engine.

**Output:**  
A string containing formatted diagnostic messages in the format: `{file}:{line}:{column}: {message}`.

**Constraints:**
- Output must be parseable by Vim's default errorformat
- Should include file path, line number, column number, and message
- Should indicate severity (error/warning)
- Should be readable by humans in terminal output

**Example:**
```
Input: [
  {
          rule: "conflictingClasses",
              message: "Conflicting padding classes: p-4 and p-6",
                  severity: "error",
                      file: "./components/Button.tsx",
                          line: 5,
                              column: 10
                                }
                                ]

                                Output: "./components/Button.tsx:5:10: error: Conflicting padding classes: p-4 and p-6"
                                ```

## Problem 7: Command-Line Interface
**Difficulty: Easy**

**Problem Statement:**  
Create a command-line interface for the linter that accepts file paths or glob patterns and outputs diagnostic messages.

**Input:**  
Command-line arguments specifying:
- File paths or glob patterns to lint
- Configuration options
- Output format options

**Output:**  
Formatted diagnostic messages for any detected issues.

**Constraints:**
- Must handle individual files or directory paths
- Must support glob patterns for file selection
- Should have reasonable default behaviors
- Should return appropriate exit codes (0 for success, non-zero for errors)

**Example:**
```
$ tailwind-lint ./src/**/*.tsx --config=./tailwind-lint.json

./src/components/Button.tsx:5:10: error: Conflicting padding classes: p-4 and p-6
./src/components/Card.tsx:12:15: warning: Consider using a design token instead of custom width
```

These problem statements outline the core challenges you'll need to solve when building your Rust-based Tailwind linter. Each can be tackled independently, making for a more manageable project while ensuring you cover all the necessary functionality.
