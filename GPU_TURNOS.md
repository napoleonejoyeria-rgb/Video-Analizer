# GPU turns (agents working in parallel)

The user edits video (DaVinci) on this PC but does NOT want harsh limits: speed matters. Rules (2026-11-03):

1. If `C:\Users\Usuario\Tools\camcuts_rs\.gpu_lock` exists and is less than 2 h old, someone else is using the GPU. Do CPU work in the meantime. If you need the GPU, create `.gpu_wait` with your name: the holder finishes its current step (at most 30 min more) and releases the lock; you take it and delete `.gpu_wait`.
2. If the lock is free, create it with your name and the time, and delete it the moment you finish. Without anyone waiting there is no time limit.
3. Usage:
   - If `Resolve.exe` (DaVinci) is running, stay moderate: about 60% of the GPU on average, and wait if it leaves less than 2 GB of VRAM free.
   - If DaVinci isn't running, use the GPU fully, but avoid long stretches pinned at 99%. Aim for 85–90% on average so the PC still responds.
   - Check for `Resolve.exe` between steps (epochs, clips) and adjust.
4. Stability: at most 2 parallel clips with YOLO-World x unless you have shown that more causes no driver reset (look for `nvlddmkm` events in the System log after each run: id 153 appeared 22 times on 30/10 between 9:07 and 9:21 pm with 4 clips, and 5 times on 29/10; also 4101).

Never open windows on the user's screen. Never write under `D:\`: read only, and outputs go under `camcuts_rs`. Never kill processes you didn't start.
