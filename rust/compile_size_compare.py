#! /usr/bin/env python3

"""
size_compare.py

Compare the compiled size of two revisions.

Usage: size_compare.py [-h|--help] [target_ref] [base_ref]

if no arguments are passed, compares HEAD to master.
if just `target_ref` is set, compares HEAD to `target_ref`
if `base_ref` is set, compares `target_ref` to `base_ref`."""

import subprocess
import sys
import os

RUST_TARGET_NAME = 'xi-core'

def main():
    if not os.path.exists('./target'):
        print("no target directory found, are you running this in \
              xi-editor/rust?")
        return 1

    args = sys.argv[1:]
    if len(args) > 2 or len(args) == 1 and args[0] in ("-h", "--help", "help"):
        return print_help()

    base_rev = "master"
    cur_rev = "HEAD"

    if len(args) == 1:
        base_rev = args[0]
    if len(args) == 2:
        cur_rev = args[1]

    if not working_directory_is_clean():
        print("Your working directory has unsaved changes. Stash or commit \
              your changes and try again")
        return 2

    return compare_revs(base_rev, cur_rev)


def print_help():
    print(__doc__)
    return 2


def resolve_rev(rev_id):
    """Given an arbitrary git revision id, returns the commit's hash"""
    try:
        output = subprocess.check_output("git rev-parse {}".format(rev_id), shell=True)
        return output.decode('utf-8').strip()
    except subprocess.CalledProcessError as err:
        print("failed to resolve rev_id {}: {}".format(rev_id, err))
        return None


def compare_revs(base_rev, cur_rev):
    base_sha = resolve_rev(base_rev)
    cur_sha = resolve_rev(cur_rev)

    if base_sha is None:
        print("could not resolve ref '{}'".format(base_rev))
        return print_help()
    if cur_sha is None:
        print("could not resolve ref '{}'".format(cur_rev))
        return print_help()
    if base_sha == cur_sha:
        print("refs are the same: {}, {}".format(base_rev, cur_rev))
        return print_help()

    cur_size = compile_size_for_commit(cur_sha)
    base_size = compile_size_for_commit(base_sha)

    base_str = base_rev + " " + base_sha[:8]
    cur_str = cur_rev + " " + cur_sha[:8]
    print_compare(base_str, cur_str, base_size, cur_size)


def print_compare(base_str, cur_str, base_size, cur_size):
    size_delta = base_size - cur_size
    delta_change = "smaller" if size_delta > 0 else "bigger"
    pad_size = max(len(base_str), len(cur_str)) + 4
    fmt_string = '{:<' + str(pad_size) + '}{}'
    print(fmt_string.format(base_str, sizeof_fmt(base_size)))
    print(fmt_string.format(cur_str, sizeof_fmt(cur_size)))
    if base_size != cur_size:
        print("{} is {} {} than {}".format(cur_str, sizeof_fmt(abs(size_delta)), delta_change, base_str))
    else:
        print("revisions are equal")


# https://stackoverflow.com/questions/1094841/reusable-library-to-get-human-readable-version-of-file-size
def sizeof_fmt(num, suffix='B'):
    for unit in ['','Ki','Mi','Gi','Ti','Pi','Ei','Zi']:
        if abs(num) < 1024.0:
            return "%3.3f%s%s" % (num, unit, suffix)
        num /= 1024.0
    return "%.1f%s%s" % (num, 'Yi', suffix)


def compile_size_for_commit(sha):
    subprocess.check_output("git checkout {}".format(sha), shell=True)
    try:
        subprocess.check_output("cargo build --release".format(sha), shell=True)
    except subprocess.CalledProcessError as err:
        return "{} exited with {}: {}".format(sha, err.returncode, err.output or "")
    ls_output = subprocess.check_output("ls -l target/release/{}".format(RUST_TARGET_NAME), shell=True)
    file_size = int(ls_output.split()[4].decode('utf-8'))
    return file_size


def working_directory_is_clean():
    try:
        subprocess.check_call("git diff-index --quiet HEAD --", shell=True)
        return True
    except subprocess.CalledProcessError:
        return False


if __name__ == "__main__":
    main()
