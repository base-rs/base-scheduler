#!/usr/bin/python3
"""Extend cargo-llvm-cov's external-source filter for Arch's remapped std paths."""
import os, sys
args=sys.argv[1:]
if args and args[0] in ['export','report','show']:
    extra='^/usr/src/debug/'
    if '-ignore-filename-regex' in args:
        index=args.index('-ignore-filename-regex')+1
        args[index]='('+args[index]+')|('+extra+')'
    else:args+=['-ignore-filename-regex',extra]
os.execv('/usr/bin/llvm-cov',['llvm-cov',*args])
