## Object Removal - Phase 2: ONNX Inference Jobs

### Goal
Implement the actual ONNX inference jobs for object removal pipeline:
1. **MobileSAM inference** - Generate mask from click coordinates on preview
2. **LaMa inference** - Generate inpainted video from video + mask
3. **Preview integration** - Click on preview → show mask overlay → apply
4. **Cache management** - Store masks and inpainted videos

### Pipeline Architecture

```
User clicks preview → MobileSAM job (click coords → mask) → Cache mask
                ↓
User clicks "Apply" → LaMa job (video + mask → inpainted video) → Cache inpainted
                ↓
Export uses inpainted video + mask for object removal
```

### Jobs to Implement

#### 1. MobileSAM Segment Job (`media_jobs.rs`)
- **Input**: Video clip path, click coordinates (normalized 0-1), project dimensions
- **Process**: 
  - Extract frame at playhead position
  - Run MobileSAM ONNX with point prompt
  - Generate binary mask (same resolution as project)
- **Output**: Mask video (FFV1 gray, same duration as clip) → `cache/objrem/<clip_id>.mkv`

#### 2. LaMa Inpainting Job (`media_jobs.rs`)
- **Input**: Video clip path, mask path
- **Process**:
  - Run LaMa ONNX frame-by-frame (video + mask → inpainted frame)
  - Encode inpainted frames to video
- **Output**: Inpainted video → `cache/objrem/<clip_id>_inpainted.mkv`

#### 3. Preview Integration (`app.rs` + `preview_player.rs`)
- Click on preview → send coordinates to MobileSAM job
- Show mask overlay on preview (semi-transparent red)
- "Apply" button triggers LaMa job
- Clear mask button

### Cache Structure
```
cache/
  objrem/
    <clip_id>.mkv              # MobileSAM mask (FFV1 gray)
    <clip_id>_inpainted.mkv    # LaMa inpainted video
```

### UI Flow (Effects Tab)
1. Enable "Object Removal" checkbox
2. Click "Select Object" button → preview enters selection mode
3. Click on object → mask appears as red overlay
3. Adjust/clear if needed
4. Click "Apply" → queues LaMa job
4. Export uses inpainted video automatically

### Technical Details

#### MobileSAM ONNX
- Input: 1024x1024 RGB + point coords (normalized)
- Output: 256x256 mask logits → upscale to project resolution
- Model: `mobile_sam.onnx` (~40MB)

#### LaMa ONNX
- Input: 512x512 RGB + mask → 512x512 inpainted
- Process frame-by-frame (can batch for speed)
- Model: `lama.onnx` (~50MB)

### Related Issues
- #94 (Object Removal MVP Infrastructure) - DONE
- This issue: ONNX Inference Jobs Implementation

### Acceptance Criteria
- [ ] Click on preview generates accurate mask
- [ ] Mask overlay visible on preview
- [ ] "Apply" generates inpainted video
- [ ] Export uses inpainted video for object removal
- [ ] Clear mask works
- [ ] Jobs respect cancellation
- [ ] Progress shown in UI