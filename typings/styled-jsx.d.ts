import 'react';

declare module 'react' {
  interface StyleHTMLAttributes<T> {
    global?: boolean;
    jsx?: boolean;
  }
}
