s/[ 	]+$//
/^#include/d
s/\([a-z]+\)->\([a-z]+\)/\2 of \1/g
/^}/{
	N
	s/\n$//
}
y/abcdef/ABCDEF/
