import type {Dispatch, Middleware} from 'redux';

import type {HyperActions, HyperState} from '../../typings/hyper';
import terms from '../terms';

// the only side effect we perform from middleware
// is to write to the react term instance directly
// to avoid a performance hit
const writeMiddleware: Middleware<{}, HyperState, Dispatch<HyperActions>> = () => (next) => (action) => {
  const typedAction = action as HyperActions;
  if (typedAction.type === 'SESSION_PTY_DATA') {
    const term = terms[typedAction.uid];
    if (term) {
      term.term.write(typedAction.data);
    }
  }
  return next(action);
};

export default writeMiddleware;
