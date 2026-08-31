"""ROI cropping for assessor input.

The assessor's cost is dominated by image input tokens — a full 1280×800 frame
is ~1.5–2k tokens per read, and count/state questions only need the pane they
ask about. Cropping the *assessor's copy* of the frame to a declared region
cuts that spend roughly in proportion to the area, while the full frame is
still grabbed, saved by ``_SavingScreen``, and retained as the forensic on a
failing step — the crop narrows what the oracle reads, never what the run
records.

Pillow is an optional dependency of the orchestrator host only (the VM agent
and CI are untouched): a journey that declares an ROI fails loudly here if it
is missing, instead of silently assessing the full frame.
"""

from __future__ import annotations

import io
from typing import Tuple

from .interfaces import Frame

# (x, y, width, height) in screen pixels, same space as Action coordinates.
Roi = Tuple[int, int, int, int]


def crop_frame(frame: Frame, roi: Roi) -> Frame:
    try:
        from PIL import Image
    except ImportError as e:  # pragma: no cover - environment-dependent
        raise RuntimeError(
            "this journey declares an ROI crop, which needs Pillow on the "
            "orchestrator host: pip3 install pillow"
        ) from e

    x, y, w, h = roi
    with Image.open(io.BytesIO(frame.data)) as img:
        box = (x, y, min(x + w, img.width), min(y + h, img.height))
        if box[0] >= box[2] or box[1] >= box[3]:
            raise ValueError(f"ROI {roi} is outside the {img.size} frame")
        out = io.BytesIO()
        img.crop(box).save(out, format="PNG")
    return Frame.of(out.getvalue(), width=box[2] - box[0], height=box[3] - box[1])
