.SUFFIXES: .dvi .ps

PAPER = main
LATEX = latex
PDFLATEX = pdflatex -file-line-error -interaction=nonstopmode
TEXFILES = *.tex
BIBFILE = references.bib


default: $(PAPER).pdf

$(PAPER).pdf: $(TEXFILES) $(ELFFILES) $(BIBFILE) makefile
	$(PDFLATEX) $(PAPER)
	bibtex $(PAPER)
	$(PDFLATEX) $(PAPER)
	$(PDFLATEX) $(PAPER)

$(PAPER).ps: $(PAPER).dvi
	dvips -P cmz -t letter -o $(PAPER).ps $(PAPER).dvi

final:	$(PAPER).pdf $(PAPER).ps
	cp $(PAPER).pdf final.pdf
	cp $(PAPER).ps final.ps

dvi: $(PAPER).dvi

ps: $(PAPER).ps

pdf: $(PAPER).pdf

preview: $(PAPER).dvi
	xdvi $(PAPER).dvi &

prevps: $(PAPER).ps
	ghostview $(PAPER).ps &

sendps: $(PAPER).ps
	sz $(PAPER).ps

sendpdf: $(PAPER).pdf
	sz $(PAPER).pdf

print: $(PAPER).ps
	lpr -Pmax $(PAPER).ps

clean:
	rm -fr *.ps *.dvi *.aux *.bbl *.blg *.log *.lof *.lot *.out *.toc $(PAPER).pdf *~ *.ent

wc:
	wc -w $(TEXFILES)

ispell:
	ispell -b -t $(TEXFILES)
