# Motion comparison

Open **Motion → Motion comparison** on Home or in the separate VRM viewer.
Register a file with the tool used and optional production notes, select
it, and choose **Play from start**. All entries use the currently selected VRM.
Changing partners keeps the library; select Play again for the new avatar.

Pause, change playback speed, scrub, or step forward/backward to inspect a pose.
The frame inspection rate is an interval chosen by the reviewer, not a claim
about the source file's original sample rate. Selecting an entry does not play it.
During comparison, procedural body motion is suppressed in both movement styles.
Stop returns control to the selected posture and idle behavior.

## Import contract

| File | Supported input |
| --- | --- |
| VRMA / GLB | Self-contained binary glTF with `VRMC_vrm_animation`; at least one humanoid rotation track |
| FBX | One playable animation, Y-up, T-pose reference, Mixamo or VRM humanoid bone names; empty takes are ignored |
| BVH | Y-up humanoid hierarchy with VRM/Mixamo bone names and a positive reference hip height |
| NPZ / custom skeleton | Convert in the producer's tools to the supported FBX/BVH/VRMA contract first |

Files are limited to 50 MB and 10 minutes. BVH additionally allows at most 36,000
samples and 200 joints. Humanoid FBX/BVH import requires mapped hips, spine,
upper arms, and upper legs. Optional mapped finger tracks are retained. Missing
fingers are not synthesized, and retargeting does not correct foot sliding,
contact, mesh penetration, or unsupported coordinate conventions.

FBX rotation conversion uses the source parent's rest-world rotation and the
inverse bone rest-world rotation. Hip translation scales by the source reference
hip height through `createVRMAnimationClip`, which also handles VRM 0.x axis
conversion. The [three-vrm Mixamo example](https://github.com/pixiv/three-vrm/blob/dev/packages/three-vrm/examples/humanoidAnimation/loadMixamoAnimation.js)
documents this coordinate conversion. FBX textures are skipped; imports do not
fetch texture files. GLB resources must be embedded.

## Authoring tools

The source picker offers Meshy, Kimodo, ARDY, Cascadeur, iClone/AccuPose,
DeepMotion, Rokoko, Flow Studio, Blender Agent Skill, dcc-mcp,
blender-claude-plugin, MotionMCP, and SAM 3D Body. It also accepts a custom name.
These identify the authoring source; they do not install or invoke a generator.
Export a supported motion from the producer or its rig-conversion workflow.
Skeleton-specific NPZ conversion and cloud generation APIs are separate work.
An image, GIF, or video of a result is not a skeletal motion file.

## Local library and reports

Files, SHA-256 hashes, source conditions, and notes are stored in
IndexedDB on the current browser/app origin. Limits are 64 files and 256 MB
combined. They survive reloads on that origin; this is not backend storage or
cross-device synchronization. Browser and desktop app storage may differ.
Keep the original motion files separately: clearing the browser/app storage
removes this library.

Notes are saved per partner. Updating one partner's notes merges
against the current stored row, preserving notes saved by another window.
The JSON report includes conditions, hashes, durations, mapped bone counts,
inspection rates and notes. It excludes file blobs; it is not a motion
backup or an automatic quality score.
