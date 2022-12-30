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
% Logging indexing metadata
\newwrite\pediaIndex
\immediate\openout\pediaIndex=pedia.txt
%
% For some things, during pass one we want to scan text as verbatim, but in pass
% 2 we want to actually evaluate the tokens. The construct
% \pediaPassOneVerbatim{AFTER}{TEXT} helps with that. It scans the braced
% expression TEXT into the toklist \pedia@maybeVerbatimToks, and then expands
% out the argument AFTER. In pass 1, TEXT will be scanned in "verbatim mode",
% while otherwise it will be scanned with the usual TeX tokenization rules. Due
% to the way that TeX's tokenization works, this is a super fragile command, and
% it has to appear as the last command of any macros that you define to use it.
\makeatletter
\newtoks{\pedia@maybeVerbatimToks}
\ifpassone
  \def\pedia@makeActiveWhitespace{%
    \catcode`\ =\active%
    \catcode`\^^I=\active%
  }%
  \begingroup%
    % for our purposes, tabs are just spaces
    \pedia@makeActiveWhitespace%
    \gdef\pedia@defineActiveWhitespace{\def {\ }\def^^I{\ }}%
  \endgroup
  \def\pediaPassOneVerbatim#1{%
    \begingroup
    % This setup derived from fancyvrb.sty rather than the plain TeX macros.
    % Recall that when this macro is expanded, everything here will already have
    % been read into a token a list, so changing the catcode of `\` doesn't
    % break our syntax here.
    %
    % Note that we are *not* changing the meaning of {} here, because we need
    % balanced delimiters for our toklist scanning. TODO: offer a mode that uses
    % different delimiters so that expressions with unbalanced braces can be
    % read. We can't do the `\verb||` trick because that requires that tokens
    % we're scanning are being expanded as we go, while we need to be saving
    % them into a toklist.
    \catcode`\\=12
    \catcode`\$=12
    \catcode`\&=12
    \catcode`\#=12
    \catcode`\%=12
    \catcode`\~=12
    \catcode`\_=12
    \catcode`\^=12
    \@noligs
    \pedia@makeActiveWhitespace
    \pedia@defineActiveWhitespace
    \def\tmp@a{\pedia@afterPassOneVerbatim #1}%
    \afterassignment\tmp@a
    \global\pedia@maybeVerbatimToks=%
  }%
  % This macro is expanded after the verbatim assignment is finished; it completes
  % the handling of the verbatim text.
  \def\pedia@afterPassOneVerbatim{%
    \endgroup
  }%
\else
  % When we're not in pass 1, the tokens are scanned with normal TeX rules so
  % that control sequences are parsed. TODO: handle any special delimiters
  % provided above.
  \def\pediaPassOneVerbatim#1{%
    \def\tmp@a{#1}%
    \afterassignment\tmp@a
    \global\pedia@maybeVerbatimToks=%
  }%
\fi
\makeatother
%
\let\justTeX=\TeX
\def\TeX{\special{tdux:cs math}\justTeX\special{tdux:ce math}}
%
% \href{URL}{TEXT}
%  This builds in a `target="_blank"` setting.
\renewcommand{\href}[2]{%
  \special{tdux:mfs a^^J%
Dtarget _blank^^J%
Dhref #1}#2\special{tdux:me a}%
}
%
% \hrefInternal{URL}{TEXT}
%  Like \href, but without the "_blank" target.
\newcommand{\hrefInternal}[2]{%
  \special{tdux:mfs a^^J%
Dhref #1}#2\special{tdux:me a}%
}
%
% \entry{TITLE}{PLAIN-TITLE}{OUTFILE}
%  Define an entry
%
%  Because the entry title needs to be parsed as verbatim code for
%  indexing, the actual implementation of this command is split.
\makeatletter
\newcommand{\entry}{%
  % This parses the first argument, places it in \pedia@maybeVerbatimToks,
  % and then evaluates \entry@tail
  \pediaPassOneVerbatim\entry@tail
}
\newcommand{\entry@tail}[2]{%
  % Render the entry title. On pass 1 it will have been parsed verbatim,
  % so any TeX constructs will appear literally. This is fine since we don't
  % actually use the pass 1 HTML!
  \the\pedia@maybeVerbatimToks%
  %
  \tduxSetupOutput{template.html}{e/#2/index.html}
  \def\pediaRelTop{../../}
  \immediate\write\pediaIndex{\string\output{e/#2/index.html}}
  \immediate\write\pediaIndex{\string\idef{entries}{#2}{}}
  % The point of all of the verbatim gymnastics: we can emit the "text" of the
  % entry as the literal TeX code that the user provided, rather than whatever
  % that code expands to.
  \immediate\write\pediaIndex{\string\itext{entries}{#2}{\the\pedia@maybeVerbatimToks}{#1}}
}
\makeatother
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
%
% \pediaEnsureRefCS{INDEX}{ENTRY}{DATATYPE}
%  Ensure that a pedia cross-referencing control string is defined to
%  `?` if it is not already defined. This will kick in in the first pass,
%  when the cross-referencing information is still being gathered. In
%  the second pass, these control strings will be defined by the
%  driver.
\newcommand{\pediaEnsureRefCS}[3]{%
  \unless\ifcsname pedia resolve**#1**#2**#3\endcsname
    \expandafter\def\csname pedia resolve**#1**#2**#3\endcsname{?}
  \fi
}
%
% \pediaLinkRef{INDEX}{ENTRY}
%  Create an internal <a> link to an index entry with a defined location
%  and textual representation.
\newcommand{\pediaLinkRef}[2]{%
  \pediaLogRef{#1}{#2}{lt}
  \pediaEnsureRefCS{#1}{#2}{loc}
  \pediaEnsureRefCS{#1}{#2}{text plain}
  \hrefInternal{%
    \pediaRelTop\csname pedia resolve**#1**#2**loc\endcsname%
   }{%
    \csname pedia resolve**#1**#2**text plain\endcsname
   }
}
%
% \e{ENTRY}
%  Create an internal <a> link to the specified encyclopedia entry.
\newcommand{\e}[1]{%
  \pediaLinkRef{entries}{#1}
}
