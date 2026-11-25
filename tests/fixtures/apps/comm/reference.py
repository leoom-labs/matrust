"""Independent scalar AWGN references (Python standard library; no MATLAB).

Gray 16-QAM is two Gray 4-PAM axes. Summing the bit decision regions gives
3/4 Q(t) + 1/2 Q(3t) - 1/4 Q(5t), t=sqrt(4*EbNo/5).
Coherent orthogonal BFSK: Q(sqrt(EbNo)); QPSK: Q(sqrt(2*EbNo)).
See Proakis, Digital Communications, coherent detection in Gaussian noise.
Run from any directory; writes no files, JSON goes to stdout.
"""
import json
import math


def q(x):
    return math.erfc(x / math.sqrt(2)) / 2


grid = list(range(0, 11, 2))
linear = [10 ** (x / 10) for x in grid]
print(json.dumps({
    "EbNo": grid,
    "QPSK": [q(math.sqrt(2 * g)) for g in linear],
    "BFSK": [q(math.sqrt(g)) for g in linear],
    "QAM16": [0.75*q(math.sqrt(0.8*g)) + 0.5*q(3*math.sqrt(0.8*g))
              - 0.25*q(5*math.sqrt(0.8*g)) for g in linear],
}, indent=2))
