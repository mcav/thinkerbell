#![allow(unused_variables)]
#![allow(dead_code)]

/// Basic structure of a Monitor (aka Server App)
///
/// Monitors are designed so that the FoxBox can offer a simple
/// IFTTT-style Web UX to let users write their own scripts. More
/// complex monitors can installed from the web from a master device
/// (i.e. the user's cellphone or smart tv).

use dependencies::{DevEnv, ExecutableDevEnv, Watcher};
use values::{Value, Range};
use compile::{UncheckedCtx, UncheckedEnv, CompiledCtx, CompiledInput, Context, DatedData}; // FIXME: Determine exactly where these definitions should go.
use compile;

use std::collections::HashMap;
use std::sync::Arc; // FIXME: Investigate if we really need so many instances of Arc. I suspect that most can be replaced by &'a.
use std::sync::mpsc::{channel, Receiver, Sender};
use std::marker::PhantomData;
use std::result::Result;
use std::result::Result::*;
use std::thread;

extern crate chrono;
use self::chrono::{DateTime, UTC};


///
/// # Definition of the AST
///


/// A Monitor Application, i.e. an application (or a component of an
/// application) executed on the server.
///
/// Monitor applications are typically used for triggering an action
/// in reaction to an event: changing temperature when night falls,
/// ringing an alarm when a door is opened, etc.
///
/// Monitor applications are installed from a paired device. They may
/// either be part of a broader application (which can install them
/// through a web/REST API) or live on their own.
pub struct Script<Ctx, Env> where Env: DevEnv, Ctx: Context {
    /// Authorization, author, description, update url, version, ...
    pub metadata: (), // FIXME: Implement

    /// Monitor applications have sets of requirements (e.g. "I need a
    /// camera"), which are allocated to actual resources through the
    /// UX. Re-allocating resources may be requested by the user, the
    /// foxbox, or an application, e.g. when replacing a device or
    /// upgrading the app.
    pub requirements: Vec<Requirement<Ctx, Env>>,

    /// Resources actually allocated for each requirement.
    /// This must have the same size as `requirements`.
    pub allocations: Vec<Resource<Ctx, Env>>,

    /// A set of rules, stating what must be done in which circumstance.
    pub rules: Vec<Trigger<Ctx, Env>>,
}

pub struct Resource<Ctx, Env> where Env: DevEnv, Ctx: Context {
    pub devices: Vec<Env::Device>,
    pub phantom: PhantomData<Ctx>,
}


/// A resource needed by this application. Typically, a definition of
/// device with some input our output capabilities.
pub struct Requirement<Ctx, Env> where Env: DevEnv, Ctx: Context {
    /// The kind of resource, e.g. "a flashbulb".
    pub kind: Env::DeviceKind,

    /// Input capabilities we need from the device, e.g. "the time of
    /// day", "the current temperature".
    pub inputs: Vec<Env::InputCapability>,

    /// Output capabilities we need from the device, e.g. "play a
    /// sound", "set luminosity".
    pub outputs: Vec<Env::OutputCapability>,
    
    pub phantom: PhantomData<Ctx>,
    // FIXME: We may need cooldown properties.
}

/// A single trigger, i.e. "when some condition becomes true, do
/// something".
pub struct Trigger<Ctx, Env> where Env: DevEnv, Ctx: Context {
    /// The condition in which to execute the trigger.
    pub condition: Conjunction<Ctx, Env>,

    /// Stuff to do once `condition` is met.
    pub execute: Vec<Statement<Ctx, Env>>,

    /*
    /// Minimal duration between two executions of the trigger.  If a
    /// duration was not picked by the developer, a reasonable default
    /// duration should be picked (e.g. 10 minutes).
    FIXME: Implement
    pub cooldown: Duration,
     */
}

/// A conjunction (e.g. a "and") of conditions.
pub struct Conjunction<Ctx, Env> where Env: DevEnv, Ctx: Context {
    /// The conjunction is true iff all of the following expressions evaluate to true.
    pub all: Vec<Condition<Ctx, Env>>,
    pub state: Ctx::ConditionState,
}

/// An individual condition.
///
/// Conditions always take the form: "data received from sensor is in
/// given range".
///
/// A condition is true if *any* of the sensors allocated to this
/// requirement has yielded a value that is in the given range.
pub struct Condition<Ctx, Env> where Env: DevEnv, Ctx: Context {
    pub input: Ctx::InputSet,
    pub capability: Env::InputCapability,
    pub range: Range,
    pub state: Ctx::ConditionState,
}


/// Stuff to actually do. In practice, this means placing calls to devices.
pub struct Statement<Ctx, Env> where Env: DevEnv, Ctx: Context {
    /// The resource to which this command applies.  e.g. "all
    /// heaters", "a single communication channel", etc.
    pub destination: Ctx::OutputSet,

    /// The action to execute on the resource.
    pub action: Env::OutputCapability,

    /// Data to send to the resource.
    pub arguments: HashMap<String, Expression<Ctx, Env>>
}

pub struct InputSet<Ctx, Env> where Env: DevEnv, Ctx: Context {
    /// The set of inputs from which to grab the value, i.e.
    /// all the inputs matching some condition.
    pub condition: Condition<Ctx, Env>,

    /// The value to grab.
    pub capability: Env::InputCapability,
}

/// A value that may be sent to an output.
pub enum Expression<Ctx, Env> where Env: DevEnv, Ctx: Context {
    /// A dynamic value, which must be read from one or more inputs.
    // FIXME: Not ready yet
    Input(InputSet<Ctx, Env>),

    /// A constant value.
    Value(Value),

    /// More than a single value.
    Vec(Vec<Expression<Ctx, Env>>)
}


///
/// # Launching and running the script
///


/// Running and controlling a single script.
pub struct Execution<Env> where Env: ExecutableDevEnv + 'static {
    command_sender: Option<Sender<ExecutionOp>>,
    phantom: PhantomData<Env>,
}

impl<Env> Execution<Env> where Env: ExecutableDevEnv + 'static {
    pub fn new() -> Self {
        Execution {
            command_sender: None,
            phantom: PhantomData,
        }
    }

    /// Start executing the script.
    ///
    /// # Errors
    ///
    /// Produces RunningError:AlreadyRunning if the script is already running.
    pub fn start<F>(&mut self, script: Script<UncheckedCtx, UncheckedEnv>, on_result: F) where F: FnOnce(Result<(), Error>) + Send + 'static {
        if self.command_sender.is_some() {
            on_result(Err(Error::RunningError(RunningError::AlreadyRunning)));
            return;
        }
        let (tx, rx) = channel();
        let tx2 = tx.clone();
        self.command_sender = Some(tx);
        thread::spawn(move || {
            match ExecutionTask::<Env>::new(script, tx2, rx) {
                Err(er) => {
                    on_result(Err(er));
                },
                Ok(mut task) => {
                    on_result(Ok(()));
                    task.run();
                }
            }
        });
    }


    /// Stop executing the script, asynchronously.
    ///
    /// # Errors
    ///
    /// Produces RunningError:NotRunning if the script is not running yet.
    pub fn stop<F>(&mut self, on_result: F) where F: Fn(Result<(), Error>) + Send + 'static {
        let result = match self.command_sender {
            None => {
                /* Nothing to stop */
                on_result(Err(Error::RunningError(RunningError::NotRunning)));
            },
            Some(ref tx) => {
                // Shutdown the application, asynchronously.
                let _ignored = tx.send(ExecutionOp::Stop(Box::new(on_result)));
            }
        };
        self.command_sender = None;
    }
}

impl<Env> Drop for Execution<Env> where Env: ExecutableDevEnv + 'static {
    fn drop(&mut self) {
        let _ignored = self.stop(|_ignored| { });
    }
}

/// A script ready to be executed.
/// Each script is meant to be executed in an individual thread.
pub struct ExecutionTask<Env> where Env: DevEnv {
    /// The current state of execution the script.
    state: Script<CompiledCtx<Env>, Env>,

    /// Communicating with the thread running script.
    tx: Sender<ExecutionOp>,
    rx: Receiver<ExecutionOp>,
}





enum ExecutionOp {
    /// An input has been updated, time to check if we have triggers
    /// ready to be executed.
    Update {index: usize, updated: DateTime<UTC>, value: Value},

    /// Time to stop executing the script.
    Stop(Box<Fn(Result<(), Error>) + Send>)
}


impl<Env> ExecutionTask<Env> where Env: ExecutableDevEnv {
    /// Create a new execution task.
    ///
    /// The caller is responsible for spawning a new thread and
    /// calling `run()`.
    fn new(script: Script<UncheckedCtx, UncheckedEnv>, tx: Sender<ExecutionOp>, rx: Receiver<ExecutionOp>) -> Result<Self, Error> {
        // Prepare the script for execution:
        // - replace instances of Input with InputDev, which map
        //   to a specific device and cache the latest known value
        //   on the input.
        // - replace instances of Output with OutputDev
        let precompiler = try!(compile::Precompiler::new(&script).map_err(|err| Error::CompileError(err)));
        let bound = try!(precompiler.rebind_script(script).map_err(|err| Error::CompileError(err)));
        
        Ok(ExecutionTask {
            state: bound,
            rx: rx,
            tx: tx
        })
    }

    /// Execute the monitoring task.
    /// This currently expects to be executed in its own thread.
    fn run(&mut self) {
        let mut watcher = Env::get_watcher();
        let mut witnesses = Vec::new();
        
        // A thread-safe indirection towards a single input state.
        // We assume that `cells` never mutates again once we
        // have finished the loop below.
        let mut cells : Vec<Arc<CompiledInput<Env>>> = Vec::new();

        // Start listening to all inputs that appear in conditions.
        // Some inputs may appear only in expressions, so we are
        // not interested in their value.
        for rule in &self.state.rules  {
            for condition in &rule.condition.all {
                for single in &*condition.input {
                    let tx = self.tx.clone();
                    cells.push(single.clone());
                    let index = cells.len() - 1;

                    witnesses.push(
                        // We can end up watching several times the
                        // same device + capability + range.  For the
                        // moment, we do not attempt to optimize
                        // either I/O (which we expect will be
                        // optimized by `watcher`) or condition
                        // checking (which we should eventually
                        // optimize, if we find out that we end up
                        // with large rulesets).
                        watcher.add(
                            &single.device,
                            &condition.capability,
                            &condition.range,
                            move |value| {
                                // One of the inputs has been updated.
                                // Update `state` and determine
                                // whether there is anything we need
                                // to do.
                                let _ignored = tx.send(ExecutionOp::Update {
                                    updated: UTC::now(),
                                    value: value,
                                    index: index
                                });
                                // If the thread is down, it is ok to ignore messages.
                            }));
                    }
            }
        }

        // Make sure that the vector never mutates past this
        // point. This ensures that our `index` remains valid for the
        // rest of the execution.
        let cells = cells;

        // FIXME: We are going to end up with stale data in some inputs.
        // We need to find out how to get rid of it.
        // FIXME(2): We now have dates.

        // Now, start handling events.
        for msg in &self.rx {
            use self::ExecutionOp::*;
            match msg {
                Stop(f) => {
                    // Leave the loop.
                    // The watcher and the witnesses will be cleaned up on exit.
                    // Any further message will be ignored.
                    f(Ok(()));
                    return;
                }

                Update {updated, value, index} => {
                    let cell = &cells[index];
                    *cell.state.write().unwrap() = Some(DatedData {
                        updated: UTC::now(),
                        data: value
                    });
                    // Note that we can unwrap() safely,
                    // as it fails only if the thread is
                    // already in panic.

                    // Find out if we should execute triggers.
                    for mut rule in &mut self.state.rules {
                        let is_met = rule.is_met();
                        if !(is_met.new && !is_met.old) {
                            // We should execute the trigger only if
                            // it was false and is now true. Here,
                            // either it was already true or it isn't
                            // false yet.
                            continue;
                        }

                        // Conditions were not met, now they are, so
                        // it is time to start executing.

                        // FIXME: We do not want triggers to be
                        // triggered too often. Handle cooldown.
                        
                        for statement in &rule.execute {
                            let _ignored = statement.eval(); // FIXME: Log errors
                        }
                    }
                }
            }
        }
    }
}

///
/// # Evaluating conditions
///

struct IsMet {
    old: bool,
    new: bool,
}

impl<Env> Trigger<CompiledCtx<Env>, Env> where Env: DevEnv {
    fn is_met(&mut self) -> IsMet {
        self.condition.is_met()
    }
}


impl<Env> Conjunction<CompiledCtx<Env>, Env> where Env: DevEnv {
    /// For a conjunction to be true, all its components must be true.
    fn is_met(&mut self) -> IsMet {
        let old = self.state.is_met;
        let mut new = true;

        for mut single in &mut self.all {
            if !single.is_met().new {
                new = false;
                // Don't break. We want to make sure that we update
                // `is_met` of all individual conditions.
            }
        }
        self.state.is_met = new;
        IsMet {
            old: old,
            new: new,
        }
    }
}


impl<Env> Condition<CompiledCtx<Env>, Env> where Env: DevEnv {
    /// Determine if one of the devices serving as input for this
    /// condition meets the condition.
    fn is_met(&mut self) -> IsMet {
        let old = self.state.is_met;
        let mut new = false;
        for single in &*self.input {
            // This will fail only if the thread has already panicked.
            let state = single.state.read().unwrap();
            let is_met = match *state {
                None => { false /* We haven't received a measurement yet.*/ },
                Some(ref data) => {
                    use values::Range::*;
                    use values::Value::*;

                    match (&data.data, &self.range) {
                        // Any always matches
                        (_, &Any) => true,
                        // Operations on bools and strings
                        (&Bool(ref b), &EqBool(ref b2)) => b == b2,
                        (&String(ref s), &EqString(ref s2)) => s == s2,

                        // Numbers. FIXME: Implement physical units.
                        (&Num(ref x), &Leq(ref max)) => x <= max,
                        (&Num(ref x), &Geq(ref min)) => min <= x,
                        (&Num(ref x), &BetweenEq{ref min, ref max}) => min <= x && x <= max,
                        (&Num(ref x), &OutOfStrict{ref min, ref max}) => x < min || max < x,

                        // Type errors don't match.
                        (&Bool(_), _) => false,
                        (&String(_), _) => false,
                        (_, &EqBool(_)) => false,
                        (_, &EqString(_)) => false,
                        (&Vec(_), _) => false,

                        // There is no such thing as a range on json or blob.
                        (&Json(_), _) |
                        (&Blob{..}, _) => false,
                    }
                }
            };
            if is_met {
                new = true;
                break;
            }
        }

        self.state.is_met = new;
        IsMet {
            old: old,
            new: new,
        }
    }
}

impl<Env> Statement<CompiledCtx<Env>, Env> where Env: ExecutableDevEnv {
    fn eval(&self) -> Result<(), Error> {
        let args = self.arguments.iter().map(|(k, v)| {
            (k.clone(), v.eval())
        }).collect();
        for output in &self.destination {
            Env::send(&output.device, &self.action, &args); // FIXME: Handle errors
        }
        return Ok(());
    }
}

impl<Env> Expression<CompiledCtx<Env>, Env> where Env: ExecutableDevEnv {
    fn eval(&self) -> Value {
        match *self {
            Expression::Value(ref v) => v.clone(),
            Expression::Input(_) => panic!("Cannot read an input in an expression yet"),
            Expression::Vec(ref vec) => {
                Value::Vec(vec.iter().map(|expr| expr.eval()).collect())
            }
        }
    }
}



#[derive(Debug)]
pub enum RunningError {
    AlreadyRunning,
    NotRunning,
}

#[derive(Debug)]
pub enum Error {
    CompileError(compile::Error),
    RunningError(RunningError),
}







