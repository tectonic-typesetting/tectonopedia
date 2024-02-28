// A list of spans for simple code rendering

export interface Span {
  cls: string,
  content: string,
}

export class SpanSet {
  spans: Span[];

  constructor() {
    this.spans = [];
  }

  append(cls: string, content: string) {
    const n = this.spans.length;

    if (n > 0 && this.spans[n - 1].cls == cls) {
      this.spans[n - 1].content += content;
    } else {
      this.spans.push({ cls, content });
    }
  }
}
