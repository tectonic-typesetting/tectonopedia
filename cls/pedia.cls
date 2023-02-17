% Copyright 2022-2023 the Tectonic Project
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
\setsansfont{texgyreheros}[%
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
\RequirePackage{fancyvrb}
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
\def\pedia@makeActiveWhitespace{%
  \catcode`\ =\active%
  \catcode`\^^I=\active%
}%
\begingroup%
  % for our purposes, tabs are just spaces
  \pedia@makeActiveWhitespace%
  \gdef\pedia@defineActiveWhitespace{\def {\ }\def^^I{\ }}%
\endgroup
\def\pediaScanVerbatim#1{%
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
  \def\tmp@a{\pedia@afterScanVerbatim #1}%
  \afterassignment\tmp@a
  \global\pedia@maybeVerbatimToks=%
}%
% This macro is expanded after the verbatim assignment is finished; it completes
% the handling of the verbatim text.
\def\pedia@afterScanVerbatim{%
  \endgroup
}%
\ifpassone
  \let\pediaPassOneVerbatim=\pediaScanVerbatim
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
\let\justLaTeX=\LaTeX
\def\LaTeX{\special{tdux:cs math}\justLaTeX\special{tdux:ce math}}
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
% \entry{SLUG}{TEX-TITLE}{PLAIN-TITLE}
%  Define an entry
%
%  Because the entry titles need to be parsed as verbatim code for
%  indexing, the actual implementation of this command is split.
\makeatletter
\newtoks\pedia@titletmp
%
\newcommand{\entry}[1]{%
  % save the slug:
  \def\tmp@b{#1}%

  % This is the stuff we can do with just the slug:
  \tduxSetupOutput{template.html}{e/#1/index.html}
  \def\pediaRelTop{../../}
  \immediate\write\pediaIndex{\string\output{e/#1/index.html}}
  \immediate\write\pediaIndex{\string\idef{entries}{#1}{}}
  % This parses the second argument (the TeX title), places it in
  % \pedia@maybeVerbatimToks, and then evaluates \entry@tailA
  \pediaPassOneVerbatim\entry@tailA
}
\newcommand{\entry@tailA}{%
  % Render the entry title. On pass 1 it will have been parsed verbatim,
  % so any TeX constructs will appear literally. This is fine since we don't
  % actually use the pass 1 HTML!
  \pediaTitle{\the\pedia@maybeVerbatimToks}%
  %
  \pedia@titletmp=\pedia@maybeVerbatimToks
  \pediaScanVerbatim\entry@tailB
}
\newcommand{\entry@tailB}{%
  % The point of all of the verbatim gymnastics: we can emit the "text" of the
  % entry as the literal TeX code that the user provided, rather than whatever
  % that code expands to.
  \immediate\write\pediaIndex{\string\itext{entries}{\tmp@b}{\the\pedia@titletmp}{\the\pedia@maybeVerbatimToks}}

  % Finally we can also set the page title
  \tduxSetTemplateVariable{pediaTitle}{\the\pedia@maybeVerbatimToks}
  \tduxSetTemplateVariable{pediaBookName}{Tectonopedia: The Reference}
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
  \immediate\write\pediaIndex{\string\iref{#1}{#2}{#3}}%
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
    \expandafter\def\csname pedia resolve**#1**#2**#3\endcsname{?}%
  \fi
}
%
% \pediaLinkRef{INDEX}{ENTRY}
%  Create an internal <a> link to an index entry with a defined location
%  and textual representation.
\newcommand{\pediaLinkRef}[2]{%
  \pediaLogRef{#1}{#2}{lt}%
  \pediaEnsureRefCS{#1}{#2}{loc}%
  \pediaEnsureRefCS{#1}{#2}{text plain}%
  \hrefInternal{%
    \pediaRelTop\csname pedia resolve**#1**#2**loc\endcsname%
   }{%
    \csname pedia resolve**#1**#2**text tex\endcsname%
   }%
}
%
% \e{ENTRY}
%  Create an internal <a> link to the specified encyclopedia entry.
\newcommand{\e}[1]{%
  \pediaLinkRef{entries}{#1}%
}
%
% \pediaTitle{TEXT}
%   Render a page title
\newcommand{\pediaTitle}[1]{%
  {\Large\textbf{\special{tdux:mfs div^^JCpedia-pagetitle}#1\special{tdux:me div}}}%
}
%
% \b is an alias for \textbf
\let\b=\textbf
%
% \i is an alias for \emph
\let\i=\emph
%
% \tex renders inline TeX code processed in a "verbatim"
%   mode. Right now it's just an alias for the fancyvrb
%   \Verb command, but we want a separate command that
%   specifically indicates that its contents are TeX
%   because in the future we might include syntax
%   highlighting, hyperlinking, etc.
\let\tex=\Verb
%
% Redefining sectioning commands to emit <h{1,2,3,4}> tags in the HTML. TODO:
% there is probably a better way to do this.
%
\RequirePackage{titlesec}
% From Section 8.2 of the titlesec docs, the settings here correspond to the
% standard formats.
%\titleformat{\chapter}[display]{\normalfont\huge\bfseries}{\chaptertitlename\ \thechapter}{20pt}{\Huge}
\titleformat{\section}{\normalfont\Large\bfseries}{\thesection}{1em}{\special{tdux:mfs h1^^JNAT}}[\special{tdux:me h1}]
\titleformat{\subsection}{\normalfont\large\bfseries}{\thesubsection}{1em}{\special{tdux:mfs h2^^JNAT}}[\special{tdux:me h2}]%
\titleformat{\subsubsection}{\normalfont\normalsize\bfseries}{\thesubsubsection}{1em}{\special{tdux:mfs h3^^JNAT}}[\special{tdux:me h3}]%
%\titleformat{\paragraph}[runin]{\normalfont\normalsize\bfseries}{\theparagraph}{1em}{}
%\titleformat{\subparagraph}[runin]{\normalfont\normalsize\bfseries}{\thesubparagraph}{1em}{}
%
% Futzing with list commands to add HTML tags.
%
\RequirePackage{enumitem}
%
\makeatletter
\newif\if@pedia@firstlistitem
\let\pedia@parAlias=\par % not sure why we need this, but here we are.
%
\setlist[itemize]{%
  label={},
  before*={%
    \let\pedia@save@item=\item
    \@pedia@firstlistitemtrue
    \def\item{%
      \if@pedia@firstlistitem
        \pedia@save@item
        \@pedia@firstlistitemfalse
      \else
        \pedia@parAlias
        \special{tdux:me li}%
        \pedia@save@item
      \fi
      \special{tdux:mfs li}%
      \leavevmode
      }%
    \special{tdux:mfs ul}%
  },
  after*={%
    \pedia@parAlias
    \special{tdux:me li}
    \special{tdux:me ul}
    \let\item=\pedia@save@item
  }%
}
%
\setlist[enumerate]{%
  label={},
  before*={%
    \let\pedia@save@item=\item
    \@pedia@firstlistitemtrue
    \def\item{%
      \if@pedia@firstlistitem
        \pedia@save@item
        \@pedia@firstlistitemfalse
      \else
        \pedia@parAlias
        \special{tdux:me li}%
        \pedia@save@item
      \fi
      \special{tdux:mfs li}%
      \leavevmode
    }%
    \special{tdux:mfs ol}%
  },
  after*={%
    \pedia@parAlias
    \special{tdux:me li}
    \special{tdux:me ol}
    \let\item=\pedia@save@item
  }%
}
\makeatother
%
% \explainer{SLUG}{TEX-TITLE}{PLAIN-TITLE}
%  Define an Explainer
%
%  Because the titles need to be parsed as verbatim code for
%  indexing, the actual implementation of this command is split.
%
%  TODO: massive duplication!!!!
\makeatletter
%
\newcommand{\explainer}[1]{%
  % save the slug:
  \def\tmp@b{#1}%

  % This is the stuff we can do with just the slug:
  \tduxSetupOutput{template.html}{explain/#1/index.html}
  \def\pediaRelTop{../../}
  \immediate\write\pediaIndex{\string\output{explain/#1/index.html}}
  \immediate\write\pediaIndex{\string\idef{explainers}{#1}{}}
  % This parses the second argument (the TeX title), places it in
  % \pedia@maybeVerbatimToks, and then evaluates \explainer@tailA
  \pediaPassOneVerbatim\explainer@tailA
}
\newcommand{\explainer@tailA}{%
  % Render the explainer title. On pass 1 it will have been parsed verbatim,
  % so any TeX constructs will appear literally. This is fine since we don't
  % actually use the pass 1 HTML!
  \pediaTitle{\the\pedia@maybeVerbatimToks}%
  %
  \pedia@titletmp=\pedia@maybeVerbatimToks
  \pediaScanVerbatim\explainer@tailB
}
\newcommand{\explainer@tailB}{%
  % The point of all of the verbatim gymnastics: we can emit the "text" of the
  % explainer as the literal TeX code that the user provided, rather than whatever
  % that code expands to.
  \immediate\write\pediaIndex{\string\itext{explainers}{\tmp@b}{\the\pedia@titletmp}{\the\pedia@maybeVerbatimToks}}

  % Finally we can also set the page title
  \tduxSetTemplateVariable{pediaTitle}{\the\pedia@maybeVerbatimToks}
  \tduxSetTemplateVariable{pediaBookName}{Tectonopedia: Explainers}
}
\makeatother
%
% "terms"
%
% TODO: add starred variant when the typeset form of the term is different than
% the plaintext.
\newcommand{\DeclareTerm}[1]{%
  \immediate\write\pediaIndex{\string\idef{terms}{#1}{}}%
  \immediate\write\pediaIndex{\string\itext{terms}{#1}{#1}{#1}}%
}
\makeatletter
\def\`{%
  \begingroup
  % Need to do \@noligs before our customization because \@noligs resets the
  % catcode of ` to be active!
  \@noligs
  \catcode`\\=12
  \catcode`\$=12
  \catcode`\&=12
  \catcode`\#=12
  \catcode`\%=12
  \catcode`\~=12
  \catcode`\_=12
  \catcode`\^=12
  \catcode`\{=12
  \catcode`\}=12
  \catcode`\`=2% = endgroup
  \pedia@makeActiveWhitespace
  \pedia@defineActiveWhitespace
  \afterassignment\pedia@termrefTail
  \global\pedia@maybeVerbatimToks=\bgroup%
}%
\def\pedia@termrefTail{%
  \endgroup
  \pediaLinkRef{terms}{\the\pedia@maybeVerbatimToks}%
}
\makeatother
