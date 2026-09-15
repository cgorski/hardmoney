"""Adapters that present hardmoney through other projects' interfaces.

* :mod:`hardmoney.compat.fecfile_validate`: the ``ValidationResult`` /
  ``ValidationError`` shape of the FEC's ``fecfile-validate`` package, over
  a whole ``.fec`` file or a single record, plus FECfile+'s
  ``line_*``-keyed Column A summary.
"""

from hardmoney.compat import fecfile_validate

__all__ = ["fecfile_validate"]
