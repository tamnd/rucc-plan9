.PS
box "guest"; arrow; box "9P"; arrow; ellipse "host"
move down 0.5 from 1st box.s
circle radius 0.3 "G"; line right 1; circle radius 0.3 "R"
spline from last circle.e right 0.5 then down 0.5 then right 0.5 ->
for i = 1 to 5 do { line from (i*0.2, -2) to (i*0.2, -2 - i*0.1) }
arc cw from (0, -3) to (1, -3)
"text" at (2, -3) ljust
.PE
.PS
define node { circle radius 0.15 $1 }
A: node("a"); move; B: node("b"); move; C: node("c")
arrow from A.e to B.w; arrow from B.e to C.w
arc -> from C.n to A.n
.PE
