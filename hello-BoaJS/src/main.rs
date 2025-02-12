use boa_engine::{Context, Source, property::Attribute, js_string};
use boa_runtime::Console;

fn main() {
    let js_code = ["new Date()", "console.log('Hello')"]; //実行したいJSのコード
    let mut context = Context::default();

    let console = Console::init(&mut context);

    context
      .register_global_property(js_string!(Console::NAME), console, Attribute::all())
      .expect("the console object shouldn't exist yet");

    
    let result = context.eval(Source::from_bytes(&js_code)); 
    // Context の eval method で JS コード評価

    match result { // match で context を拾って出力して抜ける
        Ok(res) => println!("{}", res.to_string(&mut context).unwrap().to_std_string_escaped()),
        Err(e) => eprintln!("Uncaught {e}")
    };
    
}