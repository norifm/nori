# Beat grids on live songs

AutoMix beat-matches only where an end's grid holds: confidence ≥ 0.5 and stability ≥ 0.6
(`plan::grid_ok`). On the owner's library (99 songs, mostly live rock), one straight grid over the
whole 30 s end held on 41 % of pairs with the beat model on and 22 % without it. The bands drift a few
percent and push or drag a fill, so one line through all the beats misses by more than 30 ms somewhere.

Most of those ends still have a long stretch played steady. `tempo::clean_run` finds the longest run of
16 beats or more within 30 ms of one straight grid (the latest such run in an outro, the earliest in an
intro), and the end is gridded over that run when the whole window fails. The run's span is kept per end
(`intro_grid_from_ms`, `outro_grid_to_ms`, ...), and a beat-matched mix plays inside it.

A run fitted to the tracker's beats can still be a wrong grid, steady on the off-beats. So a run grid
is kept only when the onsets fall on its beats: the mean onset peak within two frames of each beat,
over the peaks at a quarter, half and three quarters of a beat, must be at least 1.6
(`tempo::on_beat`). That is about the 15th percentile of whole-window grids that already pass. Of 60
run grids kept at 1.6, 42 agree with the other tracker (classical or model) and about 5 look wrong.
Without the check, 62 % of run grids agreed, against 90 % of whole-window grids.

| pairs gridded | whole window | with runs |
|---|---|---|
| beat model | 40 % | 65 % |
| classical | 22 % | 43 % |

Over every pair, DJ now beat-matches 22.3 % of mixes (12.5 % before glides and runs). More gridded
pairs also meet the tempo and key gates, so more of them cut on a drop where they used to fade.
