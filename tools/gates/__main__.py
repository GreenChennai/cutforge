"""python -m tools.gates M0…A7 —— 与 python tools/gates/gate.py 等价的模块入口。"""
import sys

from . import main

if __name__ == "__main__":
    sys.exit(main())
