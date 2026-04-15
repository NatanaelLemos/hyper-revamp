import React, {forwardRef} from 'react';

import type {TabsProps} from '../../typings/hyper';
import {decorate, getTabProps} from '../utils/plugins';

import Tab_ from './tab';

const Tab = decorate(Tab_, 'Tab');
const isMac = /Mac/.test(navigator.userAgent);

const Tabs = forwardRef<HTMLElement, TabsProps>((props, ref) => {
  const {tabs = [], borderColor, onChange, onClose, fullScreen, openNewTab, defaultProfile, profiles} = props;

  const accentFor = (profileName?: string) => {
    if (!profileName) return undefined;
    return profiles?.find((p) => p.name === profileName)?.color;
  };

  const hide = !isMac && tabs.length === 1;
  const tabsVisible = tabs.length > 1;

  return (
    <nav className={`tabs_nav ${hide ? 'tabs_hiddenNav' : ''}`} ref={ref}>
      <div className="tabs_dragStrip" />
      {props.customChildrenBefore}
      {tabs.length === 1 && isMac ? <div className="tabs_title">{tabs[0].title}</div> : null}
      {tabs.length > 1 ? (
        <>
          <ul key="list" className={`tabs_list ${fullScreen && isMac ? 'tabs_fullScreen' : ''}`}>
            {tabs.map((tab, i) => {
              const {uid, title, isActive, hasActivity} = tab;
              const tabProps = getTabProps(tab, props, {
                text: title === '' ? 'Shell' : title,
                isFirst: i === 0,
                isLast: tabs.length - 1 === i,
                borderColor,
                isActive,
                hasActivity,
                accentColor: accentFor(tab.profile),
                onSelect: onChange.bind(null, uid),
                onClose: onClose.bind(null, uid)
              });
              return <Tab key={`tab-${uid}`} {...tabProps} />;
            })}
          </ul>
          {isMac && (
            <div
              key="shim"
              style={{borderColor}}
              className={`tabs_borderShim ${fullScreen ? 'tabs_borderShimUndo' : ''}`}
            />
          )}
        </>
      ) : null}
      <div
        title="New Tab"
        className={`new_tab ${tabsVisible ? 'tabs_visible' : 'tabs_hidden'}`}
        onClick={() => openNewTab(defaultProfile)}
        onDoubleClick={(e) => e.stopPropagation()}
      >
        +
      </div>
      {props.customChildren}

      <style jsx>{`
        .new_tab {
          background: transparent;
          color: #fff;
          border-left-style: solid;
          border-bottom-style: solid;
          border-left-width: 1px;
          border-bottom-width: 1px;
          cursor: pointer;
          font-size: 16px;
          height: 34px;
          line-height: 34px;
          padding: 0 16px;
          text-align: center;
          -webkit-user-select: none;
          -webkit-app-region: no-drag;
        }

        .tabs_visible {
          border-color: ${borderColor};
        }

        .tabs_hidden {
          border-color: transparent;
          position: absolute;
          right: 0px;
        }

        .tabs_hidden:hover {
          border-color: ${borderColor};
        }
      `}</style>

      <style jsx>{`
        .tabs_nav {
          font-size: 12px;
          height: 34px;
          line-height: 34px;
          vertical-align: middle;
          color: #9b9b9b;
          cursor: default;
          position: relative;
          -webkit-user-select: none;
          -webkit-app-region: ${isMac ? 'drag' : ''};
          top: ${isMac ? '0px' : '34px'};
          display: flex;
          flex-flow: row;
        }

        .tabs_hiddenNav {
          display: none;
        }

        .tabs_title {
          text-align: center;
          color: #fff;
          overflow: hidden;
          text-overflow: ellipsis;
          white-space: nowrap;
          padding-left: 76px;
          padding-right: 76px;
          flex-grow: 1;
        }

        .tabs_list {
          max-height: 34px;
          display: flex;
          flex-flow: row;
          margin-left: ${isMac ? '76px' : '0'};
          flex-grow: 1;
        }

        .tabs_fullScreen {
          margin-left: -1px;
        }

        .tabs_borderShim {
          position: absolute;
          width: 76px;
          bottom: 0;
          border-color: #ccc;
          border-bottom-style: solid;
          border-bottom-width: 1px;
        }

        .tabs_borderShimUndo {
          border-bottom-width: 0px;
        }

        .tabs_dragStrip {
          position: absolute;
          top: 0;
          left: 0;
          right: 0;
          height: 10px;
          -webkit-app-region: drag;
          z-index: 50;
        }
      `}</style>
    </nav>
  );
});

Tabs.displayName = 'Tabs';

export default Tabs;
