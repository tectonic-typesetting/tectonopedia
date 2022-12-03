// We don't use Gulp "for real" -- this is a small hack to generate prettified
// HTML files to make it easier to understand the output that Tectonic is
// creating. In a quick check I couldn't find a way to do HTML prettification
// with Parcel.

const gulp = require("gulp");
const formatHtml = require("gulp-format-html");

function views () {
  return gulp.src("build/**/*.html").pipe(formatHtml()).pipe(gulp.dest("pretty"));
}

exports.default = gulp.series(views);
