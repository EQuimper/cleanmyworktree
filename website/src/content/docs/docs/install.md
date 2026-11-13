---
title: Install
description: Install cmw with Homebrew or the shell installer, and add cmw cd to your shell.
---

cmw is a single binary for macOS and Linux on ARM64 and x86-64.

## Homebrew

Homebrew uses the same tap on macOS and Linux.

```sh
brew install equimper/tap/cmw
```

Upgrade with `brew upgrade cmw`.

## Shell installer

Each [GitHub release](https://github.com/EQuimper/cleanmyworktree/releases) includes an installer script that puts `cmw` in `~/.cargo/bin`:

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/EQuimper/cleanmyworktree/releases/latest/download/cmw-installer.sh | sh
```

## From source

With a Rust toolchain installed:

```sh
cargo install --git https://github.com/EQuimper/cleanmyworktree
```

## Add `cmw cd`

A program can't change its parent shell's directory, so `cmw cd` comes from a small shell function. Add the line for your shell to its startup file:

```sh
# ~/.zshrc
eval "$(cmw shell-init zsh)"

# ~/.bashrc
eval "$(cmw shell-init bash)"
```

```fish
# ~/.config/fish/config.fish
cmw shell-init fish | source
```

Then jump into any worktree by branch, slug or directory name:

```sh
cmw cd feat/login
```

The function only handles `cd`. Every other subcommand goes straight to the binary.

## Check it works

```sh
cmw --version
cmw ls
```
