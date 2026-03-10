#!/usr/bin/env python3
import subprocess, sys
cmd=["/Users/dmcgregsauce/.codex/heiwa/bin/heiwax","observe","tail"]+sys.argv[1:]
raise SystemExit(subprocess.call(cmd))
