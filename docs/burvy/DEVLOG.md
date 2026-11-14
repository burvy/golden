11/14/26, 10:30AM - Set up a lot of docs and README skeleton. Ray should be working on getting localhost working and the sun function by now

<<<<<<< HEAD
11/14/26, 12:20PM - core/horizon (task 012, built by the horizon agent, reviewed by Boss): horizon(grid, &HorizonQuery) gives the skyline from an eye point. For each azimuth, walk out one grid cell at a time and keep the steepest slope (h - eye)/d; elev = atan(slope). No data in a direction = flat (elev 0, dist null). Flat earth on purpose (0.18 m drop at 1.5 km). ray_max_below adds an early exit (stop once even the grid's tallest building further out couldn't beat the best slope). It's only safe when the eye is below the grid top, which a review catch fixed and a test now covers. 14 tests. ~14 ms per full 360 deg on a 3 km grid.
=======
11/14/26, 12:47PM - Vexcels, Sun stuff, everything for the frontend person should be done. However, we ran into an issue, being that Vexcels doesn't give us data for around Princeton.
>>>>>>> 7b90e5100d88cbeb9c9f8c20705ce83ad64311fe
