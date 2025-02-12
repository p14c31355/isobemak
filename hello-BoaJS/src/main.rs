use boa_engine::{Context, Source, property::Attribute, js_string};
use boa_runtime::Console;

trait JsCode { // JSのコードをtrait impl にしてみた
  fn date(&self) -> String;
  fn hello(&mut self) -> String;
}

impl dyn JsCode { // dyn は dynamic 
  fn date(&self) -> String {
    "new Date()";
  }

  fn hello(&mut self) -> String {
    "console.log('Hello')";
  }
}

fn js_code<T: JsCode>(f: &mut T) {

    let mut context = Context::default();

    let console = Console::init(&mut context);

    context
      .register_global_property(js_string!(Console::NAME), console, Attribute::all())
      .expect("the console object shouldn't exist yet");

    
    let result = context.eval(Source::(&f.date(), &f.hello())); 
    // Context の eval method で JS コード評価

    match result { // match で context を拾って出力して抜ける
        Ok(res) => println!("{}", res.to_string(&mut context).unwrap().to_std_string_escaped()),
        Err(e) => eprintln!("Uncaught {e}")
    };
    
}

fn main() {
  let js = fn js_code();
}