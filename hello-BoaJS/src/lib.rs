use boa_engine::{Context, Source, property::Attribute, js_string};
use boa_runtime::Console;

pub fn log() {
  let js_code2 = "console.log('Hello')";
  let mut context2 = Context::default();

  let console2 = Console::init(&mut context2);

  context2
      .register_global_property(js_string!(Console::NAME), console2, Attribute::all())
      .expect("the console object shouldn't exist yet");
  
  let result2 = context2.eval(Source::from_bytes(js_code2));

  match result2 {
    Ok(res) => println!("{}", res.to_string(&mut context2).unwrap().to_std_string_escaped()),
    Err(e) => eprintln!("Uncaught {e}")
  };
}