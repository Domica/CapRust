## Object Removal / Video Inpainting

### Goal
Allow users to remove unwanted objects/people from video clips.

### MVP Approach (Phase 1)
- **Segmentation:** MobileSAM (lightweight ONNX, ~40MB) - click-to-select object
- **Inpainting:** ffmpeg-based (minterpolate + nlmeans + optical flow) - runs on CPU
- **UI:** Click on preview → auto mask → preview result → apply to clip

### Phase 2 (Future)
- ProPainter / E2FGVI (GPU, higher quality)
- Brush-based mask editing, feather, expand

### Dependencies
- Add MobileSAM model to AI Models registry (ModelKind::Segmenter)
- Download URL: https://github.com/ChaoningZhang/MobileSAM/blob/master/weights/mobile_sam.onnx
- Integrate with existing u2netp pipeline for mask refinement

### UI Flow
1. Right-click clip → "Remove Object" (or Effects tab button)
2. Click on object in preview → SAM generates mask
3. Show mask overlay, allow adjustment
4. Click "Apply" → runs inpainting during export

### Technical Notes
- Too slow for real-time preview → runs during export/render (like AI Enhance)
- Reuses existing model download infrastructure
- Mask stored per-clip in ProjectState (similar to chroma key mask)

### Related
- Uses existing u2netp for mask refinement
- Similar to #39 (AI Enhance) integration pattern