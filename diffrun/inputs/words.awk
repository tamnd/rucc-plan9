# Count the words of the input, with the length of each.
{
	for(i = 1; i <= NF; i++){
		w = tolower($i)
		gsub(/[^a-z0-9_]/, "", w)
		if(w == "")
			continue
		n[w]++
		len[length(w)]++
	}
}
END {
	for(w in n)
		if(n[w] > 3)
			printf "%d %s\n", n[w], w
	for(l in len)
		printf "len %d %d\n", l, len[l]
}
