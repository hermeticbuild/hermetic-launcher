#!/usr/bin/env bash
# Fixture entrypoint for launcher.entrypoint() analysis tests. It's never executed;
# it just needs to be a real executable target so ctx.executable.entrypoint resolves.
echo "fake entrypoint"
