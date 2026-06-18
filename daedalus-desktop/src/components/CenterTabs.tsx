import type { WorkTab } from "../types";
import { tabIcon } from "../utils/view";

type Props = {
  tabs: WorkTab[];
  activeTabId: string;
  onActivateTab: (tabId: string) => void;
  onCloseTab: (tabId: string) => void;
  onNewTab: () => void;
  onToggleLeft: () => void;
  onToggleRight: () => void;
};

export function CenterTabs(props: Props) {
  return (
    <header className="tabbar">
      <button className="layout-toggle" title="隐藏/展开左侧栏" onClick={props.onToggleLeft}>◧</button>
      <div className="tabs">
        {props.tabs.map((tab) => (
          <button
            key={tab.id}
            className={`tab ${tab.id === props.activeTabId ? "active" : ""}`}
            onClick={() => props.onActivateTab(tab.id)}
          >
            <span className="tab-type">{tabIcon(tab.kind)}</span>
            <span className="tab-title">{tab.title}</span>
            <span
              className="close-tab"
              onClick={(event) => {
                event.stopPropagation();
                props.onCloseTab(tab.id);
              }}
            >
              ×
            </span>
          </button>
        ))}
      </div>
      <button className="new-tab-button" onClick={props.onNewTab}>＋</button>
      <button className="layout-toggle" title="隐藏/展开右侧工具区" onClick={props.onToggleRight}>◨</button>
    </header>
  );
}
