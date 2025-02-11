use boa_engine::{Context, Source};

fn main() {
    let js_code = "new Date()"; //実行したいJSのコード
    let mut context = Context::default();
    let result = context.eval(Source::from_bytes(js_code)); 
    // Context の eval method で JS コード評価

    match result {
        Ok(res) => println!("{}", res.to_string(&mut context).unwrap().to_std_string_escaped()),
        Err(e) => eprintln!("Uncaught {e}")
    };
}