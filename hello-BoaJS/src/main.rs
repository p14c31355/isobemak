use boa_engine::{Context, Source, property::Attribute, js_string};
use boa_runtime::Console;

fn main() {
    let js_code = "new Date()"; //実行したいJSのコード
    let js_code2 = "console.log('Hello')";
    let mut context = Context::default();
    
    let result = context.eval(Source::from_bytes(js_code)); 
    // Context の eval method で JS コード評価

    match result { // match で context を拾って出力して抜ける
        Ok(res) => println!("{}", res.to_string(&mut context).unwrap().to_std_string_escaped()),
        Err(e) => eprintln!("Uncaught {e}")
    };
    
    fn log();
}