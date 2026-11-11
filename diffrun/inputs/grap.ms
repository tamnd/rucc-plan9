.G1
frame ht 2 wid 3
label left "time (ms)"
label bot "units"
coord x 0, 100 y 0, 50
ticks bot from 0 to 100 by 20
draw solid
1 2
10 6
20 9
40 17
60 23
80 35
100 48
new dotted
for i from 0 to 100 by 10 do { next at i, i / 3 + 2 }
.G2
.G1
frame ht 1.5 wid 2.5
coord log x
plot "rtt" at 100, 3
plot "boot" at 1000, 5
plot "build" at 10000, 2
.G2
