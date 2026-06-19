変更耐性と拡張性を持つことを厳守して
- Dependency Injection: 具象クラスを直接 new せず、外から渡せ。（依存の整理）
- Rule of Three: 同じコードが3回出てくるまで、共通化・抽象化するな。（YAGNIの徹底）
- Refactor-ready Test: テストがないコードは、負債とみなす。（品質の担保）
- Performance Escape Hatch: 性能上重要な箇所（Hot Path）に限り、コメントで理由を明記した上で上記ルールを逸脱することを許可する。
- 実装途中で疑問があれば質問しろ。

v1.0518