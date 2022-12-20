% Copyright 2022 the Tectonic Project
% Licensed under the MIT License
%
% Tectonopedia document class
%
\NeedsTeXFormat{LaTeX2e}[1995/12/01]
\ProvidesClass{pedia}[2022/10/30 Tectonopedia document class]
%
\ExecuteOptions{}
\ProcessOptions
%
\LoadClass{article}
%
\RequirePackage{fontspec}
\setmainfont{texgyrepagella}[%
  Ligatures = TeX,
  Extension = .otf,
  UprightFont = *-regular,
  BoldFont = *-bold,
  ItalicFont = *-italic,
  BoldItalicFont = *-bolditalic,
]
\setmonofont{SourceCodePro}[%
  Ligatures = TeX,
  Extension = .otf,
  UprightFont = *-Regular,
  BoldFont = *-Bold,
  ItalicFont = *-RegularIt,
  BoldItalicFont = *-BoldIt,
]
%
\RequirePackage{amsmath}
\RequirePackage{unicode-math}
\setmathfont{texgyrepagella-math.otf}
%
\RequirePackage{hyperref}
%
% Get the generic TDUX macros
\input{tdux.tex}
%
\tduxAddTemplate{template.html}
\tduxSetTemplateVariable{tduxDocumentTitle}{Set tduxDocumentTitle}
% Logging indexing metadata
\newwrite\pediaIndex
\openout\pediaIndex=pedia.txt
%
\let\justTeX=\TeX
\def\TeX{\special{tdux:cs math}\justTeX\special{tdux:ce math}}
%
% \href{URL}{TEXT}
\renewcommand{\href}[2]{%
\special{tdux:mfs a^^J%
Dtarget _blank^^J%
Dhref #1}#2\special{tdux:me a}%
}
%
% \entry{TITLE}{PLAIN-TITLE}{OUTFILE}
%  Define an entry
\newcommand{\entry}[3]{%
  #1
  \tduxSetupOutput{template.html}{e/#3/index.html}
  \write\pediaIndex{\string\output{e/#3/index.html}}
  \write\pediaIndex{\string\idef{entries}{#3}{}}
  \write\pediaIndex{\string\itext{entries}{#3}{#1}{#2}}
  }
%
% \pediaLogRef{INDEX}{ENTRY}{FLAGS}
%  Log a reference to an index entry in the metadata file.
%  This low-level command does not actally expand to any content!
%  An "l" in the flags indicates that the entry's definition location
%  must be defined. A "t" in the flags indicates that the entry's
%  text must be defined.
\newcommand{\pediaLogRef}[3]{%
  \write\pediaIndex{\string\iref{#1}{#2}{#3}}
}
