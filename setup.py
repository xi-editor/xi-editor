#!/usr/bin/env python3
import sys
import io
import re
from setuptools import setup, find_packages

setup(
    name='xi-editor',
    description='A modern editor with a backend written in Rust.',
    author='Raph Levien',
    packages=find_packages(),
    tests_require=['pytest']
)