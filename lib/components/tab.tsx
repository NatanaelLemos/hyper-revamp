import React, {forwardRef, useRef} from 'react';

import type {TabProps} from '../../typings/hyper';
import {ipcRenderer} from '../utils/ipc';

const DRAG_THRESHOLD = 4; // pixels — anything smaller is treated as a click

const Tab = forwardRef<HTMLLIElement, TabProps>((props, ref) => {
  const downPos = useRef<{sx: number; sy: number; wx: number; wy: number} | null>(null);
  const didDrag = useRef(false);

  const handleMouseDown = async (event: React.MouseEvent) => {
    if (event.nativeEvent.which !== 1) return;
    // Start tracking — if the mouse ends up moving, we'll drag the window
    // instead of treating this as a click.
    didDrag.current = false;
    const [wx, wy] = (await ipcRenderer.invoke('window:get-position')) as [number, number];
    downPos.current = {sx: event.screenX, sy: event.screenY, wx, wy};

    const onMove = (e: MouseEvent) => {
      if (!downPos.current) return;
      const dx = e.screenX - downPos.current.sx;
      const dy = e.screenY - downPos.current.sy;
      if (!didDrag.current && Math.hypot(dx, dy) < DRAG_THRESHOLD) return;
      didDrag.current = true;
      ipcRenderer.send('window:set-position', downPos.current.wx + dx, downPos.current.wy + dy);
    };
    const onUp = () => {
      window.removeEventListener('mousemove', onMove);
      window.removeEventListener('mouseup', onUp);
      downPos.current = null;
    };
    window.addEventListener('mousemove', onMove);
    window.addEventListener('mouseup', onUp);
  };

  const handleClick = (event: React.MouseEvent) => {
    const isLeftClick = event.nativeEvent.which === 1;
    if (didDrag.current) return; // this was a drag, not a click
    if (isLeftClick && !props.isActive) {
      props.onSelect();
    }
  };

  const handleMouseUp = (event: React.MouseEvent) => {
    const isMiddleClick = event.nativeEvent.which === 2;

    if (isMiddleClick) {
      props.onClose();
    }
  };

  const {isActive, isFirst, isLast, borderColor, hasActivity, accentColor} = props;

  return (
    <>
      <li
        onClick={props.onClick}
        onMouseDown={handleMouseDown}
        style={{borderColor}}
        className={`tab_tab ${isFirst ? 'tab_first' : ''} ${isActive ? 'tab_active' : ''} ${
          isFirst && isActive ? 'tab_firstActive' : ''
        } ${hasActivity ? 'tab_hasActivity' : ''}`}
        ref={ref}
      >
        {props.customChildrenBefore}
        {accentColor ? <span className="tab_accent" style={{background: accentColor}} /> : null}
        <i className="tab_closeLeft" onClick={props.onClose} title="Close tab">
          ×
        </i>
        <span
          className={`tab_text ${isLast ? 'tab_textLast' : ''} ${isActive ? 'tab_textActive' : ''}`}
          onClick={handleClick}
          onMouseUp={handleMouseUp}
        >
          <span title={props.text} className="tab_textInner">
            {props.text}
          </span>
        </span>
        {props.customChildren}
      </li>

      <style jsx>{`
        .tab_tab {
          color: #ccc;
          border-color: #ccc;
          border-bottom-width: 1px;
          border-bottom-style: solid;
          border-left-width: 1px;
          border-left-style: solid;
          list-style-type: none;
          flex-grow: 1;
          position: relative;
          -webkit-app-region: no-drag;
        }

        .tab_tab:hover {
          color: #ccc;
        }

        .tab_first {
          border-left-width: 0;
          padding-left: 1px;
        }

        .tab_firstActive {
          border-left-width: 1px;
          padding-left: 0;
        }

        .tab_active {
          color: #fff;
          border-bottom-width: 0;
        }
        .tab_active:hover {
          color: #fff;
        }

        .tab_hasActivity {
          color: #50e3c2;
        }

        .tab_hasActivity:hover {
          color: #50e3c2;
        }

        .tab_text {
          transition: color 0.2s ease;
          height: 34px;
          display: block;
          width: 100%;
          position: relative;
          overflow: hidden;
        }

        .tab_textInner {
          position: absolute;
          left: 24px;
          right: 24px;
          top: 0;
          bottom: 0;
          text-align: center;
          text-overflow: ellipsis;
          white-space: nowrap;
          overflow: hidden;
        }

        .tab_closeLeft {
          position: absolute;
          left: 7px;
          top: 10px;
          width: 14px;
          height: 14px;
          line-height: 14px;
          text-align: center;
          font-size: 14px;
          font-style: normal;
          border-radius: 100%;
          color: #e9e9e9;
          cursor: pointer;
          z-index: 1;
          -webkit-app-region: no-drag;
        }

        .tab_closeLeft:hover {
          background-color: rgba(255, 255, 255, 0.13);
          color: #fff;
        }

        .tab_closeLeft:active {
          background-color: rgba(255, 255, 255, 0.1);
          color: #909090;
        }

        .tab_accent {
          position: absolute;
          top: 0;
          left: 0;
          right: 0;
          height: 2px;
          pointer-events: none;
        }

      `}</style>
    </>
  );
});

Tab.displayName = 'Tab';

export default Tab;
