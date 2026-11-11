# usage: analyze.sh A.tsv B.tsv ; mean improvement per candidate (vs the block's nice-0 reference), then transfer metrics
imp() { awk -F'\t' '$3==0 && $4!="NA"{t[$1","$2]=$4; b[$1]=1} END{for(k in b){r=t[k",0"]; for(n=3;n<=19;n++){ if(n==3||n==6||n==10||n==19){s[n]+=(r-t[k",".n])/r; c[n]++}}} for(n in s) printf "%d %.4f\n", n, 100*s[n]/c[n]}' "$1" | sort -k2 -nr; }
