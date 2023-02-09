<template>
  <div class="content-aligned">
    <h1>Keyboard Shortcuts</h1>
    <ul class="keylist">
      <li>
        <span class="d"><span>Show/hide this help</span></span><span class="k">?</span>
      </li>
      <li>
        <span class="d"><span>Show/hide search</span></span><span class="k">/</span>
      </li>
      <li>
        <span class="d"><span>Close tool</span></span><span class="k">Esc</span>
      </li>
    </ul>
  </div>
</template>

<style lang="scss" scoped>
.keylist {
  width: 100%;
  padding: 0;
  margin-top: 2rem;

  li {
    width: 100%;
    margin: 2px 0;
    border: 1px solid var(--searchbar-border-color);
    border-radius: 3px;
    padding: 2px;
    box-sizing: border-box;

    display: flex;
    align-items: center;

    line-height: 18px;

    .d {
      flex: 1;
      height: 18px;

      span {
        display: inline-block;
        line-height: normal;
        vertical-align: middle;
      }
    }

    .k {
      flex: 0 30px;
      height: 18px;
      font-family: "tduxMono";
      font-size: 0.875em;
      text-align: center;

      span {
        display: inline-block;
        line-height: normal;
        vertical-align: middle;
      }
    }
  }
}
</style>