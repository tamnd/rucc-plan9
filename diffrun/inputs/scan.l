%{
/* A scanner of a small language, for lex in diffrun. */
int nline = 1;
%}
D	[0-9]
L	[a-zA-Z_]
%%
"/*"([^*]|"*"+[^*/])*"*"+"/"	{ printf("COMMENT\n"); }
{L}({L}|{D})*	{ printf("NAME %s\n", yytext); }
{D}+"."{D}*([eE][-+]?{D}+)?	{ printf("FLOAT %s\n", yytext); }
0[xX][0-9a-fA-F]+	{ printf("HEX %s\n", yytext); }
{D}+	{ printf("INT %s\n", yytext); }
\"([^"\\\n]|\\.)*\"	{ printf("STRING %s\n", yytext); }
"<<="|">>="|"&&"|"||"|"=="|"!="|"<="|">="|"->"|"++"|"--"	{ printf("OP %s\n", yytext); }
[-+*/%<>=!&|^~?:;,.(){}\[\]]	{ printf("PUNCT %c\n", yytext[0]); }
\n	{ nline++; }
[ \t]+	;
.	{ printf("BAD %c at %d\n", yytext[0], nline); }
%%
int
yywrap(void)
{
	return 1;
}
