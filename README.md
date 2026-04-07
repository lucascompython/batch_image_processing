# Batch Image Processing

Based on FastStone Image Viewer.

## Features

- Be able to convert images in batch to pdf/jpg
- Be able to add text to the images (e.g. file name)
- Be able to rotate the images
- Numbering mode for fast per-image bike-number tagging

## OCR setup (optional, Numbering mode)

The app can suggest motorcycle numbers using local ONNX OCR.

1. Put OCR assets in `models/ppocrv5/` (or set `BIP_OCR_MODEL_DIR`).
2. Ensure this folder contains:
   - `det.onnx`
   - `rec.onnx`
   - `PP-LCNet_x0_25_textline_ori.onnx`
   - `PP-LCNet_x1_0_doc_ori.onnx`
   - `ppocrv5_dict.txt`

If these files are missing, the app still runs, but OCR suggestions stay disabled.

### Sticker template assist (Numbering mode)

You can load an event sticker template from the Numbering toolbar (`Sticker Template` button).
When loaded, OCR first tries to locate that sticker in each photo and prioritizes the number area
of that sticker before falling back to full-image OCR. This helps reduce confusion with old-event
stickers that appear in the same frame.
