import {createStore, applyMiddleware} from 'redux';
import {thunk as reduxThunk} from 'redux-thunk';
import type {ThunkMiddleware} from 'redux-thunk';

import type {HyperState, HyperActions} from '../../typings/hyper';
import rootReducer from '../reducers/index';
import effects from '../utils/effects';
import * as plugins from '../utils/plugins';

import writeMiddleware from './write-middleware';

const thunk = reduxThunk as ThunkMiddleware<HyperState, HyperActions>;

const configureStoreForProd = () =>
  createStore(rootReducer, applyMiddleware(thunk, plugins.middleware, thunk, writeMiddleware, effects));

export default configureStoreForProd;
