import includeLanguages from '@theme-original/prism-include-languages';
import includeShine from '../prism-shine';

export default function prismIncludeLanguages(Prism) {
  includeLanguages(Prism);
  includeShine(Prism);
}
