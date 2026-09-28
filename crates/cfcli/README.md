# cfcli

The file plumbing the two binaries share, so that they walk, read and name
files the same way: `walk` (a directory in path order, honouring
`.gitignore`, the global gitignore, `.git/info/exclude` and `.ignore`,
hidden entries skipped, symlinks not followed), `git_changes` (the files
`--git staged|unstaged|all` selects: `git` from `PATH` finds the
repository root, then lists the staged, unstaged and untracked files with
`-C <root>`; deduplicated, in path order, regular files of the working tree
only, named relative to the current directory when under it and joined to
the root otherwise; git's message, or the IO error of running it, as the
error), `Dedupe` (a file named
twice, by absolute path, is taken once), `Input` (a file or stdin, its name
in messages, `read_utf8`: bytes that are not UTF-8 are an error, never
replaced) and `io_message` (an IO error without its `(os error N)`
suffix). It is used by `cfformat-cli` (the `cfformat` binary) and `cfvet`,
which choose the files they keep and print their own messages; it depends
on `ignore` alone, so `cfvet` shares the walk without pulling in the
formatter; `git_changes` runs git as a process (`std::process::Command`),
no library.
