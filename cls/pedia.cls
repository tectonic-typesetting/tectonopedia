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
% \entry{TITLE}{OUTFILE}
%  Define an entry
\makeatletter
\newcommand{\entry}[2]{%
  #1
  \tduxSetupOutput{template.html}{#2.html}
  \write\@auxout{\gdef\string\pediaEntrypoint{#2.html}}
}
\makeatother